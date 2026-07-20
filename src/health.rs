use crate::state::{AppState, Backend};
use std::sync::Arc;

/// Probe a single backend and update its `healthy` flag.
///
/// A probe is a `GET {health_url}` returning 2xx; anything else marks the
/// backend down so candidate selection skips it. This module is the sole
/// authority on each backend's `healthy` flag.
pub async fn probe_backend(state: &AppState, backend: &Backend) {
    let url = backend.health_url();
    let result = state
        .http
        .get(&url)
        .bearer_auth(&backend.cfg.api_key)
        .timeout(state.health_check_timeout)
        .send()
        .await;

    let (up, reason) = match &result {
        Ok(r) if r.status().is_success() => (true, String::new()),
        Ok(r) => (false, format!("upstream returned status {}", r.status())),
        Err(e) => (false, format!("request failed: {e}")),
    };
    let was = backend.is_healthy();
    backend.set_healthy(up);
    if was != up {
        if up {
            tracing::warn!(
                backend = %backend.cfg.name,
                url = %url,
                healthy = up,
                "backend health changed"
            );
        } else {
            tracing::warn!(
                backend = %backend.cfg.name,
                url = %url,
                healthy = up,
                reason = %reason,
                "backend health changed"
            );
        }
    }
}

/// Run one health-check pass against every configured backend.
pub async fn check_all(state: &AppState) {
    for backend in &state.backends {
        probe_backend(state, backend).await;
    }
}

/// Background loop performing periodic health probes against every backend.
///
/// The first probe pass runs at process startup (before listen); this loop only
/// handles periodic updates after that.
pub async fn run_health_checks(state: Arc<AppState>) {
    loop {
        tokio::time::sleep(state.health_check_interval).await;
        check_all(&state).await;
    }
}
