use crate::config;
use crate::state::{ApiKeyInfo, AppState, ModelInfo};
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct UpstreamModels {
    #[serde(default)]
    data: Vec<UpstreamModel>,
}

#[derive(Debug, Deserialize)]
struct UpstreamModel {
    id: String,
}

/// Build the target URL for a backend's `/models` endpoint.
fn models_url(base: &str) -> String {
    // base already includes the version path, e.g. https://api.openai.com/v1
    format!("{base}/models")
}

/// Fetch the upstream model id list for one backend. Returns `(ids, ok)`.
pub async fn fetch_models(
    http: &reqwest::Client,
    backend: &crate::state::Backend,
) -> Result<Vec<String>, String> {
    let url = models_url(&backend.base_url);
    let resp = http
        .get(&url)
        .bearer_auth(&backend.cfg.api_key)
        .timeout(std::time::Duration::from_secs(
            backend.cfg.timeout_secs.max(1),
        ))
        .send()
        .await
        .map_err(|e| format!("request /models: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "upstream /models returned status {}",
            resp.status()
        ));
    }

    let parsed: UpstreamModels = resp
        .json()
        .await
        .map_err(|e| format!("decoding /models JSON: {e}"))?;
    Ok(parsed.data.into_iter().map(|m| m.id).collect())
}

/// One full refresh pass: query every backend, update per-backend caches, and
/// republish the merged, deduplicated list.
pub async fn refresh_once(state: &Arc<AppState>) {
    let mut infos: Vec<ModelInfo> = Vec::new();

    for backend in &state.backends {
        match fetch_models(&state.http, backend).await {
            Ok(ids) => {
                // The health-check task is the single authority on the
                // `healthy` flag; the model refresh only updates the cached
                // model list so routing decisions and the /models view stay
                // accurate.
                *backend.raw_models.write().expect("raw_models poisoned") = ids.clone();

                let names = backend.display_models();
                for name in names {
                    infos.push(ModelInfo {
                        id: name,
                        owned_by: backend.cfg.name.clone(),
                    });
                }
            }
            Err(e) => {
                // Keep the last-known-good model list (do NOT wipe it): an
                // empty list would make `serves()` reject models even though
                // the backend may be momentarily healthy. The health checker
                // separately gates routing via the `healthy` flag.
                tracing::warn!(
                    backend = %backend.cfg.name,
                    error = %e,
                    "failed to refresh model list from backend (keeping cached list)"
                );
            }
        }
    }

    // Deduplicate by id (first backend wins; multiple backends may expose the
    // same alias/original name).
    let mut seen = std::collections::HashSet::new();
    infos.retain(|m| seen.insert(m.id.clone()));

    let count = infos.len();
    state.models_list.store(Arc::new(infos));
    tracing::info!("model list refreshed: {count} unique models");
}

/// Background loop refreshing the merged model list.
///
/// The first refresh is performed at process startup (before listen); this
/// loop only handles the periodic updates after that.
pub async fn run_models_refresh(state: Arc<AppState>) {
    loop {
        tokio::time::sleep(state.models_refresh_interval).await;
        refresh_once(&state).await;
    }
}

/// Render the merged model list, restricted to `allowed` when the caller's
/// API key has a model allowlist (empty/`None` means unrestricted).
fn render(state: &AppState, allowed: Option<&[String]>) -> Value {
    let list = state.models_list.load();
    let data: Vec<Value> = list
        .iter()
        .filter(|m| allowed.is_none_or(|a| config::model_allowed(a, &m.id)))
        .map(|m| {
            json!({
                "id": m.id,
                "object": "model",
                "created": 0,
                "owned_by": m.owned_by,
            })
        })
        .collect();
    json!({ "object": "list", "data": data })
}

/// `GET /v1/models` and `GET /models`. When the caller authenticated with an
/// API key that has a model allowlist, only those models are listed.
pub async fn get_models(
    State(state): State<Arc<AppState>>,
    key_info: Option<Extension<Arc<ApiKeyInfo>>>,
) -> Response {
    let allowed = key_info
        .as_ref()
        .map(|Extension(info)| info.allowed_models.as_slice());
    (StatusCode::OK, Json(render(&state, allowed))).into_response()
}
