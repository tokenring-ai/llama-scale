use crate::config;
use crate::state::{ApiKeyInfo, AppState, ModelContext, ModelInfo};
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// One model id from an upstream `/models` body, plus any context it reported.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscoveredModel {
    id: String,
    context: Option<ModelContext>,
}

/// Build the target URL for a backend's `/models` endpoint.
fn models_url(base: &str) -> String {
    // base already includes the version path, e.g. https://api.openai.com/v1
    format!("{base}/models")
}

/// A positive JSON number. Zero, fractions, and non-numeric values are not a window.
fn positive_u64(value: Option<&Value>) -> Option<u64> {
    let Value::Number(number) = value? else {
        return None;
    };
    if let Some(n) = number.as_u64() {
        return (n > 0).then_some(n);
    }
    let n = number.as_f64()?;
    (n.is_finite() && n >= 1.0 && n <= u64::MAX as f64 && n.fract() == 0.0).then_some(n as u64)
}

/// Served window from one upstream model object.
///
/// The first field that is set wins: vLLM's `max_model_len`, llama.cpp's
/// runtime `meta.n_ctx`, then the other names servers use for the same window.
/// `meta.n_ctx_train` is the trained length and stays out of this chain.
fn served_len(model: &Value) -> Option<u64> {
    let meta = model.get("meta");
    positive_u64(model.get("max_model_len"))
        .or_else(|| positive_u64(meta.and_then(|m| m.get("n_ctx"))))
        .or_else(|| positive_u64(model.get("context_length")))
        .or_else(|| positive_u64(model.get("max_context_length")))
        .or_else(|| positive_u64(model.get("max_input_tokens")))
}

/// Context fields worth keeping from one upstream model object.
fn context_from_upstream(model: &Value) -> Option<ModelContext> {
    let meta = model.get("meta");
    let context = ModelContext {
        len: served_len(model),
        n_ctx_train: positive_u64(meta.and_then(|m| m.get("n_ctx_train"))),
        n_vocab: positive_u64(meta.and_then(|m| m.get("n_vocab"))),
    };
    (context.len.is_some() || context.n_ctx_train.is_some() || context.n_vocab.is_some())
        .then_some(context)
}

/// Decode an upstream `/models` body. Entries with no string `id` are skipped.
fn parse_upstream_models(body: &Value) -> Result<Vec<DiscoveredModel>, String> {
    let data = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "decoding /models JSON: missing data array".to_string())?;
    let mut models = Vec::with_capacity(data.len());
    for item in data {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        models.push(DiscoveredModel {
            id: id.to_string(),
            context: context_from_upstream(item),
        });
    }
    Ok(models)
}

/// Context for a name in the merged list. An alias uses its real upstream id.
fn context_for<'a>(
    name: &str,
    aliases: &HashMap<String, String>,
    discovered: &'a [DiscoveredModel],
) -> Option<&'a ModelContext> {
    let source = aliases.get(name).map(String::as_str).unwrap_or(name);
    discovered
        .iter()
        .find(|model| model.id == source)
        .and_then(|model| model.context.as_ref())
}

/// Fetch the upstream model list for one backend.
async fn fetch_models(
    http: &reqwest::Client,
    backend: &crate::state::Backend,
) -> Result<Vec<DiscoveredModel>, String> {
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

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("decoding /models JSON: {e}"))?;
    parse_upstream_models(&body)
}

/// One full refresh pass: query every backend, update per-backend caches, and
/// republish the merged, deduplicated list.
pub async fn refresh_once(state: &Arc<AppState>) {
    let mut infos: Vec<ModelInfo> = Vec::new();

    for backend in &state.backends {
        match fetch_models(&state.http, backend).await {
            Ok(discovered) => {
                // The health-check task is the single authority on the
                // `healthy` flag; the model refresh only updates the cached
                // model list so routing decisions and the /models view stay
                // accurate.
                *backend.raw_models.write().expect("raw_models poisoned") =
                    discovered.iter().map(|model| model.id.clone()).collect();

                let names = backend.display_models();
                for name in names {
                    infos.push(ModelInfo {
                        id: name.clone(),
                        owned_by: backend.cfg.name.clone(),
                        context: context_for(&name, &backend.aliases, &discovered).cloned(),
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
        .map(model_object)
        .collect();
    json!({ "object": "list", "data": data })
}

/// One model object. A known served window is written as both `max_model_len`
/// and `meta.n_ctx`. Trained length and vocab ride along in `meta` when the
/// upstream sent them.
fn model_object(model: &ModelInfo) -> Value {
    let mut object = Map::new();
    object.insert("id".to_string(), json!(model.id));
    object.insert("object".to_string(), json!("model"));
    object.insert("created".to_string(), json!(0));
    object.insert("owned_by".to_string(), json!(model.owned_by));
    if let Some(context) = &model.context {
        if let Some(len) = context.len {
            object.insert("max_model_len".to_string(), json!(len));
        }
        let mut meta = Map::new();
        if let Some(len) = context.len {
            meta.insert("n_ctx".to_string(), json!(len));
        }
        if let Some(train) = context.n_ctx_train {
            meta.insert("n_ctx_train".to_string(), json!(train));
        }
        if let Some(vocab) = context.n_vocab {
            meta.insert("n_vocab".to_string(), json!(vocab));
        }
        if !meta.is_empty() {
            object.insert("meta".to_string(), Value::Object(meta));
        }
    }
    Value::Object(object)
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

#[cfg(test)]
mod tests {
    use super::{context_for, model_object, parse_upstream_models, DiscoveredModel};
    use crate::state::{ModelContext, ModelInfo};
    use serde_json::{json, Value};
    use std::collections::HashMap;

    fn parsed(body: &str) -> Vec<DiscoveredModel> {
        parse_upstream_models(&serde_json::from_str(body).unwrap()).unwrap()
    }

    fn object_for(context: Option<ModelContext>) -> Value {
        model_object(&ModelInfo {
            id: "m".into(),
            owned_by: "mac".into(),
            context,
        })
    }

    #[test]
    fn max_model_len_is_copied_into_meta_n_ctx() {
        let found = parsed(
            r#"{"object":"list","data":[{"id":"glm","object":"model","max_model_len":1048576}]}"#,
        );
        assert_eq!(found[0].id, "glm");
        assert_eq!(found[0].context.as_ref().unwrap().len, Some(1_048_576));
        let rendered = object_for(found[0].context.clone());
        assert_eq!(rendered["max_model_len"], json!(1_048_576));
        assert_eq!(rendered["meta"]["n_ctx"], json!(1_048_576));
        assert!(rendered["meta"].get("n_ctx_train").is_none());
    }

    #[test]
    fn meta_n_ctx_is_copied_into_max_model_len() {
        let found = parsed(
            r#"{"data":[{"id":"qwen","meta":{"n_ctx":262144,"n_ctx_train":262144,"n_vocab":248320}}]}"#,
        );
        let rendered = object_for(found[0].context.clone());
        assert_eq!(rendered["max_model_len"], json!(262_144));
        assert_eq!(rendered["meta"]["n_ctx"], json!(262_144));
        assert_eq!(rendered["meta"]["n_ctx_train"], json!(262_144));
        assert_eq!(rendered["meta"]["n_vocab"], json!(248_320));
    }

    #[test]
    fn other_served_names_fill_both_fields() {
        for (body, expect) in [
            (r#"{"data":[{"id":"a","context_length":8192}]}"#, 8192),
            (r#"{"data":[{"id":"a","max_context_length":4096}]}"#, 4096),
            (
                r#"{"data":[{"id":"a","max_input_tokens":200000}]}"#,
                200_000,
            ),
            (r#"{"data":[{"id":"a","max_model_len":1024.0}]}"#, 1024),
        ] {
            let found = parsed(body);
            let rendered = object_for(found[0].context.clone());
            assert_eq!(rendered["max_model_len"], json!(expect), "{body}");
            assert_eq!(rendered["meta"]["n_ctx"], json!(expect), "{body}");
        }
    }

    #[test]
    fn served_fields_outrank_later_names_and_training_context() {
        let found = parsed(
            r#"{"data":[{"id":"a","max_model_len":32768,"context_length":1,"meta":{"n_ctx":65536,"n_ctx_train":131072}}]}"#,
        );
        assert_eq!(found[0].context.as_ref().unwrap().len, Some(32_768));
        assert_eq!(
            found[0].context.as_ref().unwrap().n_ctx_train,
            Some(131_072)
        );

        let train_only = parsed(r#"{"data":[{"id":"a","meta":{"n_ctx_train":131072}}]}"#);
        let rendered = object_for(train_only[0].context.clone());
        assert!(rendered.get("max_model_len").is_none());
        assert!(rendered["meta"].get("n_ctx").is_none());
        assert_eq!(rendered["meta"]["n_ctx_train"], json!(131_072));
    }

    #[test]
    fn missing_or_unusable_context_is_omitted() {
        let found = parsed(
            r#"{"data":[{"id":"plain"},{"id":"","max_model_len":8},{"max_model_len":8},{"id":"zero","max_model_len":0},{"id":"bad","max_model_len":"1048576"}]}"#,
        );
        assert_eq!(
            found.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["plain", "zero", "bad"]
        );
        assert!(found.iter().all(|m| m.context.is_none()));
        let rendered = object_for(None);
        assert!(rendered.get("max_model_len").is_none());
        assert!(rendered.get("meta").is_none());
        assert_eq!(rendered["owned_by"], json!("mac"));
    }

    #[test]
    fn an_alias_uses_the_real_models_context() {
        let discovered = parsed(
            r#"{"data":[{"id":"gpt-4-turbo","max_model_len":128000},{"id":"other","max_model_len":1}]}"#,
        );
        let aliases = HashMap::from([("gpt-4".to_string(), "gpt-4-turbo".to_string())]);
        assert_eq!(
            context_for("gpt-4", &aliases, &discovered).unwrap().len,
            Some(128_000)
        );
        assert!(context_for("missing", &HashMap::new(), &discovered).is_none());
    }

    #[test]
    fn a_body_without_data_is_rejected() {
        let body = serde_json::json!({"models": []});
        assert!(parse_upstream_models(&body).is_err());
    }
}
