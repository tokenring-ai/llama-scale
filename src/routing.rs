use crate::error::RouteError;
use crate::state::AppState;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Compute a stable session id for sticky routing.
///
/// The signature is `sha256(api_key || model || first_message_json)`. The first
/// message (usually the system prompt) identifies a conversation without any
/// client cooperation, so repeated turns of the same conversation stick to the
/// backend that already has their context. When there is no `messages` array
/// (e.g. embeddings), only `(api_key, model)` contribute, which still keeps a
/// given user+model pinned to one backend.
pub fn session_id(api_key: &str, model: &str, body: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(api_key.as_bytes());
    hasher.update([0u8]);
    hasher.update(model.as_bytes());
    hasher.update([0u8]);

    let first_msg = body
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|a| a.first())
        .map(|m| m.to_string());
    match first_msg {
        Some(json) => hasher.update(json.as_bytes()),
        None => hasher.update([0u8]),
    }

    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Indices of backends that serve `model`, are healthy, and are not saturated
/// (`max_connections` not reached). Saturation is re-checked atomically when a
/// request is actually acquired; this filter is a best-effort pre-screen.
pub fn healthy_candidates(state: &AppState, model: &str) -> Vec<usize> {
    state
        .backends
        .iter()
        .enumerate()
        .filter(|(_, b)| b.is_healthy() && !b.is_saturated() && b.serves(model))
        .map(|(i, _)| i)
        .collect()
}

/// Ordered candidate list for a request:
/// 1. The session-affinity backend (if any healthy, non-saturated candidate) first.
/// 2. Remaining candidates ordered by ascending `fallback` tier, then by
///    ascending in-flight connection count within each tier.
///
/// New (unpinned) sessions therefore prefer the lowest `fallback` value among
/// healthy backends, using least-connections as a tiebreaker. Higher fallback
/// tiers are only attempted when every backend at a lower tier is unavailable
/// or fails during the request. Saturated backends are skipped so load spills
/// to the next candidate.
pub fn ordered_candidates(state: &AppState, model: &str, session_id: &str) -> Vec<usize> {
    let mut cands = healthy_candidates(state, model);
    if cands.is_empty() {
        return Vec::new();
    }

    sort_by_fallback_and_connections(state, &mut cands);

    if let Some(idx) = state.session_cache.get(session_id) {
        if let Some(pos) = cands.iter().position(|&x| x == idx) {
            let affinity = cands.remove(pos);
            cands.insert(0, affinity);
        }
    }
    cands
}

fn sort_by_fallback_and_connections(state: &AppState, cands: &mut [usize]) {
    cands.sort_by(|&a, &b| {
        let ba = &state.backends[a];
        let bb = &state.backends[b];
        ba.cfg
            .fallback
            .cmp(&bb.cfg.fallback)
            .then_with(|| ba.active_count().cmp(&bb.active_count()))
    });
}

/// Look up the effective request model name, returning `MissingModel` if absent.
pub fn request_model(body: &Value) -> Result<&str, RouteError> {
    body.get("model")
        .and_then(|m| m.as_str())
        .ok_or(RouteError::MissingModel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BackendConfig, ModelAlias};
    use crate::state::Backend;

    fn make_state(backends: Vec<Arc<Backend>>) -> Arc<AppState> {
        let http = reqwest::Client::new();
        let api_keys = Arc::new(["k".to_string()].into_iter().collect());
        Arc::new(AppState {
            backends,
            api_keys,
            auth_enabled: true,
            session_cache: Cache::builder().build(),
            models_list: ArcSwap::from_pointee(Vec::new()),
            http,
            models_refresh_interval: Duration::from_secs(30),
            health_check_interval: Duration::from_secs(15),
            health_check_timeout: Duration::from_secs(5),
        })
    }

    use crate::state::AppState;
    use arc_swap::ArcSwap;
    use moka::sync::Cache;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;

    fn base_cfg(name: &str, fallback: u32) -> BackendConfig {
        BackendConfig {
            name: name.into(),
            url: "https://example.com/v1".into(),
            api_key: "k".into(),
            timeout_secs: 120,
            stream_idle_timeout_secs: 120,
            stream_timeout_secs: 0,
            max_connections: 0,
            health_path: "/models".into(),
            fallback,
            model_aliases: vec![],
        }
    }

    fn backend_with_aliases(name: &str, aliases: &[(&str, &str)]) -> Arc<Backend> {
        let mut cfg = base_cfg(name, 0);
        cfg.model_aliases = aliases
            .iter()
            .map(|(a, r)| ModelAlias {
                alias: a.to_string(),
                real: r.to_string(),
            })
            .collect();
        let backend = Backend::from_cfg(cfg).unwrap();
        // from_cfg starts unhealthy; tests that exercise routing mark healthy.
        backend.set_healthy(true);
        Arc::new(backend)
    }

    fn backend_with_models(name: &str, models: &[&str]) -> Arc<Backend> {
        backend_with_models_and_fallback(name, models, 0)
    }

    fn backend_with_models_and_fallback(
        name: &str,
        models: &[&str],
        fallback: u32,
    ) -> Arc<Backend> {
        let backend = Backend::from_cfg(base_cfg(name, fallback)).unwrap();
        *backend.raw_models.write().expect("raw_models poisoned") =
            models.iter().map(|s| s.to_string()).collect();
        // from_cfg starts unhealthy; mark up so candidate selection includes it.
        backend.set_healthy(true);
        Arc::new(backend)
    }

    fn backend_with_models_and_max(
        name: &str,
        models: &[&str],
        max_connections: u64,
    ) -> Arc<Backend> {
        let mut cfg = base_cfg(name, 0);
        cfg.max_connections = max_connections;
        let backend = Backend::from_cfg(cfg).unwrap();
        *backend.raw_models.write().expect("raw_models poisoned") =
            models.iter().map(|s| s.to_string()).collect();
        backend.set_healthy(true);
        Arc::new(backend)
    }

    #[test]
    fn session_id_is_stable_for_same_conversation() {
        let body: Value = serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role":"system","content":"you are helpful"}]
        });
        let a = session_id("key1", "gpt-4", &body);
        let b = session_id("key1", "gpt-4", &body);
        assert_eq!(a, b);
    }

    #[test]
    fn session_id_differs_by_user_and_message() {
        let body1: Value = serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role":"system","content":"one"}]
        });
        let body2: Value = serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role":"system","content":"two"}]
        });
        assert_ne!(
            session_id("k", "gpt-4", &body1),
            session_id("k", "gpt-4", &body2)
        );
        assert_ne!(
            session_id("k1", "gpt-4", &body1),
            session_id("k2", "gpt-4", &body1)
        );
    }

    #[test]
    fn aliases_gate_served_models() {
        let b = backend_with_aliases("openai", &[("gpt-4", "gpt-4-turbo")]);
        assert!(b.serves("gpt-4"));
        assert!(!b.serves("gpt-4-turbo")); // alias is the public name only
        assert!(!b.serves("gpt-3.5"));
    }

    #[test]
    fn no_aliases_uses_raw_models() {
        let b = backend_with_models("local", &["llama-3", "qwen2"]);
        assert!(b.serves("llama-3"));
        assert!(!b.serves("absent-model"));
    }

    #[test]
    fn healthy_candidates_filters_health_and_model() {
        let b0 = backend_with_models("a", &["m1"]);
        b0.set_healthy(false);
        let b1 = backend_with_models("b", &["m1", "m2"]);
        let st = make_state(vec![b0, b1]);

        assert_eq!(healthy_candidates(&st, "m1"), vec![1]); // index 0 unhealthy
        assert_eq!(healthy_candidates(&st, "m2"), vec![1]);
        assert_eq!(healthy_candidates(&st, "absent"), Vec::<usize>::new());
    }

    #[test]
    fn ordered_candidates_least_connections_then_affinity() {
        let b0 = backend_with_models("a", &["m1"]);
        let b1 = backend_with_models("b", &["m1"]);
        b1.active.fetch_add(1, Ordering::Relaxed); // b1 now busier
        let st = make_state(vec![b0, b1]);

        // New session -> least-loaded first (index 0).
        let order = ordered_candidates(&st, "m1", "no-such-session");
        assert_eq!(order.first().copied(), Some(0));
    }

    #[test]
    fn affinity_backend_moves_to_front() {
        let b0 = backend_with_models("a", &["m1"]);
        let b1 = backend_with_models("b", &["m1"]);
        let st = make_state(vec![b0, b1]);

        let sid = "deadbeef";
        st.session_cache.insert(sid.to_string(), 1);

        let order = ordered_candidates(&st, "m1", sid);
        assert_eq!(order.first().copied(), Some(1));
    }

    #[test]
    fn unpinned_requests_prefer_lowest_fallback_tier() {
        let primary = backend_with_models_and_fallback("primary", &["m1"], 0);
        let backup = backend_with_models_and_fallback("backup", &["m1"], 1);
        backup.active.fetch_add(5, Ordering::Relaxed); // busier, but higher fallback
        let st = make_state(vec![primary, backup]);

        let order = ordered_candidates(&st, "m1", "new-session");
        assert_eq!(order, vec![0, 1]);
    }

    #[test]
    fn unpinned_requests_use_least_connections_within_fallback_tier() {
        let b0 = backend_with_models_and_fallback("a", &["m1"], 0);
        let b1 = backend_with_models_and_fallback("b", &["m1"], 0);
        b1.active.fetch_add(2, Ordering::Relaxed);
        let st = make_state(vec![b0, b1]);

        let order = ordered_candidates(&st, "m1", "new-session");
        assert_eq!(order.first().copied(), Some(0));
    }

    #[test]
    fn pinned_session_keeps_affinity_over_fallback_tier() {
        let primary = backend_with_models_and_fallback("primary", &["m1"], 0);
        let backup = backend_with_models_and_fallback("backup", &["m1"], 1);
        let st = make_state(vec![primary, backup]);

        let sid = "pinned-to-backup";
        st.session_cache.insert(sid.to_string(), 1);

        let order = ordered_candidates(&st, "m1", sid);
        assert_eq!(order.first().copied(), Some(1));
    }

    #[test]
    fn higher_fallback_tier_used_when_lower_tier_unhealthy() {
        let primary = backend_with_models_and_fallback("primary", &["m1"], 0);
        primary.set_healthy(false);
        let backup = backend_with_models_and_fallback("backup", &["m1"], 1);
        let st = make_state(vec![primary, backup]);

        let order = ordered_candidates(&st, "m1", "new-session");
        assert_eq!(order, vec![1]);
    }

    #[test]
    fn saturated_backends_are_skipped() {
        let limited = backend_with_models_and_max("limited", &["m1"], 1);
        limited.active.store(1, Ordering::Relaxed); // at capacity
        let spare = backend_with_models_and_max("spare", &["m1"], 4);
        let st = make_state(vec![limited, spare]);

        assert_eq!(healthy_candidates(&st, "m1"), vec![1]);
        assert_eq!(ordered_candidates(&st, "m1", "s"), vec![1]);
    }

    #[test]
    fn saturated_affinity_backend_is_skipped() {
        let limited = backend_with_models_and_max("limited", &["m1"], 1);
        limited.active.store(1, Ordering::Relaxed);
        let spare = backend_with_models("spare", &["m1"]);
        let st = make_state(vec![limited, spare]);

        let sid = "pinned-to-full";
        st.session_cache.insert(sid.to_string(), 0);

        // Affinity target is saturated; only the spare remains.
        assert_eq!(ordered_candidates(&st, "m1", sid), vec![1]);
    }

    #[test]
    fn zero_max_connections_is_unlimited() {
        let b = backend_with_models_and_max("open", &["m1"], 0);
        b.active.store(10_000, Ordering::Relaxed);
        let st = make_state(vec![b]);
        assert_eq!(healthy_candidates(&st, "m1"), vec![0]);
    }
}
