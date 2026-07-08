use crate::access_log;
use crate::auth;
use crate::error::{ApiError, RouteError};
use crate::metrics::{self, TokenTracker};
use crate::routing;
use crate::state::{AppState, Backend};
use async_stream::stream;
use axum::body::Body;
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures::StreamExt;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// RAII guard that decrements a backend's in-flight counter on drop. Lives
/// inside the streaming response body so the count stays accurate for the full
/// duration of a streamed (SSE) response, not just the headers.
struct ConnGuard {
    active: Arc<AtomicU64>,
}

impl ConnGuard {
    fn new(active: Arc<AtomicU64>) -> Self {
        active.fetch_add(1, Ordering::Relaxed);
        Self { active }
    }
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Headers that must not be forwarded in either direction (hop-by-hop per
/// RFC 7230 plus a few that the proxy re-derives).
fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
            | "host"
            | "content-length"
    )
}

fn strip_request_headers(headers: &mut HeaderMap) {
    headers.remove(axum::http::header::AUTHORIZATION);
    let names: Vec<HeaderName> = headers
        .keys()
        .filter(|k| is_hop_by_hop(k))
        .cloned()
        .collect();
    for n in names {
        headers.remove(&n);
    }
}

fn strip_response_headers(headers: &mut HeaderMap) {
    let names: Vec<HeaderName> = headers
        .keys()
        .filter(|k| is_hop_by_hop(k))
        .cloned()
        .collect();
    for n in names {
        headers.remove(&n);
    }
}

/// `GET /` -> small identification blob.
pub async fn root() -> axum::Json<Value> {
    axum::Json(serde_json::json!({
        "service": "llama-scale",
        "status": "ok",
        "endpoints": { "models": "/v1/models", "chat": "/v1/chat/completions" }
    }))
}

/// `GET /healthz` -> liveness probe (unauthenticated).
pub async fn healthz() -> &'static str {
    "ok"
}

/// Fallback handler that proxies any unmatched `/v1/*` path to a chosen backend.
pub async fn proxy(
    State(state): State<Arc<AppState>>,
    method: Method,
    OriginalUri(original): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let path = original.path().to_string();
    if !(path == "/v1" || path.starts_with("/v1/")) {
        return Ok(ApiError::not_found(format!("unknown path: {path}")).into_response());
    }

    let body_json: Value = serde_json::from_slice(&body).map_err(|_| {
        ApiError::bad_request(
            "request body is not valid JSON; the router requires a JSON body to read 'model'",
        )
    })?;
    let model = routing::request_model(&body_json).map_err(RouteError::into_api)?;

    // The caller's bearer (or empty when auth is disabled) gives per-user
    // isolation for the conversation hash.
    let api_key = auth::bearer_token(&headers).unwrap_or_default();
    let sid = routing::session_id(&api_key, model, &body_json);

    let order = routing::ordered_candidates(&state, model, &sid);
    if order.is_empty() {
        return Err(RouteError::NoBackendForModel(model.to_string()).into_api());
    }

    // Everything after the router's own `/v1` prefix is appended to the
    // backend's configured base URL (which already includes `/v1`).
    let suffix = path.strip_prefix("/v1").unwrap_or(&path);
    let query = original
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();

    let mut last_err = String::from("no backend attempted");
    for idx in order {
        let backend = state.backend(idx);
        match forward_once(
            &state.http,
            &backend,
            &method,
            suffix,
            &query,
            &headers,
            &body,
            &body_json,
            model,
        )
        .await
        {
            Ok(mut resp) => {
                state.session_cache.insert(sid.clone(), idx);
                resp.extensions_mut()
                    .insert(access_log::RoutedBackend(backend.cfg.name.clone()));
                return Ok(resp);
            }
            Err(e) => {
                tracing::warn!(
                    backend = %backend.cfg.name,
                    error = %e,
                    "backend attempt failed; trying next candidate"
                );
                last_err = e;
            }
        }
    }

    tracing::error!(model = %model, error = %last_err, "all candidate backends failed");
    Err(RouteError::AllBackendsFailed(model.to_string()).into_api())
}

/// Attempt one backend. Returns:
/// - `Ok(response)` for any HTTP response received from upstream (including
///   4xx/5xx), which is passed straight through to the client.
/// - `Err` only when no response was received at all (connect/timeout failure),
///   signaling the caller to retry on the next candidate.
#[allow(clippy::too_many_arguments)]
async fn forward_once(
    http: &reqwest::Client,
    backend: &Backend,
    method: &Method,
    suffix: &str,
    query: &str,
    req_headers: &HeaderMap,
    raw_body: &Bytes,
    body_json: &Value,
    model: &str,
) -> Result<Response, String> {
    let target = format!("{}{suffix}{query}", backend.base_url);

    let send_body: Bytes = if let Some(real) = backend.aliases.get(model) {
        let mut bj = body_json.clone();
        bj["model"] = Value::String(real.clone());
        match serde_json::to_vec(&bj) {
            Ok(v) => Bytes::from(v),
            Err(e) => return Err(format!("failed to re-encode body after alias rewrite: {e}")),
        }
    } else {
        raw_body.clone()
    };

    let mut fwd = req_headers.clone();
    strip_request_headers(&mut fwd);
    if !backend.cfg.api_key.is_empty() {
        let hv = HeaderValue::from_str(&format!("Bearer {}", backend.cfg.api_key))
            .map_err(|e| format!("invalid backend api key: {e}"))?;
        fwd.insert(axum::http::header::AUTHORIZATION, hv);
    }

    let guard = ConnGuard::new(backend.active.clone());
    let upstream_started = Instant::now();

    let upstream = http
        .request(method.clone(), &target)
        .headers(fwd)
        .body(send_body)
        .send()
        .await
        .map_err(|e| format!("upstream request failed: {e}"))?;

    let status = upstream.status();
    let mut out_headers = upstream.headers().clone();
    strip_response_headers(&mut out_headers);

    let mut upstream_stream = upstream.bytes_stream();
    let s = stream! {
        let _guard = guard; // held until the generator completes or is dropped
        let mut ttfb_recorded = false;
        let mut token_tracker = TokenTracker::new();
        while let Some(chunk) = upstream_stream.next().await {
            match chunk {
                Ok(bytes) => {
                    if !ttfb_recorded {
                        metrics::record_time_to_first_byte(upstream_started.elapsed());
                        ttfb_recorded = true;
                    }
                    token_tracker.observe_chunk(&bytes);
                    yield Ok::<_, std::io::Error>(bytes);
                }
                Err(e) => {
                    yield Err(std::io::Error::other(e.to_string()));
                    break;
                }
            }
        }
        token_tracker.finish();
        let tokens = token_tracker.total_tokens();
        if tokens > 0 {
            let duration = token_tracker
                .stream_duration()
                .filter(|d| !d.is_zero())
                .unwrap_or_else(|| upstream_started.elapsed());
            metrics::record_token_generation(tokens, duration);
        }
    };

    let mut resp = Response::new(Body::from_stream(s));
    *resp.status_mut() = status;
    *resp.headers_mut() = out_headers;
    Ok(resp)
}
