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
use std::time::{Duration, Instant};

/// RAII guard that decrements a backend's in-flight counter on drop. Lives
/// inside the streaming response body so the count stays accurate for the full
/// duration of a streamed (SSE) response, not just the headers.
struct ConnGuard {
    active: Arc<AtomicU64>,
}

impl ConnGuard {
    /// Atomically reserve one in-flight slot. Returns `None` when
    /// `max_connections > 0` and the backend is already at capacity.
    /// `max_connections == 0` means unlimited.
    fn try_acquire(active: &Arc<AtomicU64>, max_connections: u64) -> Option<Self> {
        loop {
            let cur = active.load(Ordering::Relaxed);
            if max_connections > 0 && cur >= max_connections {
                return None;
            }
            match active.compare_exchange_weak(
                cur,
                cur.saturating_add(1),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(Self {
                        active: Arc::clone(active),
                    })
                }
                Err(_) => continue,
            }
        }
    }
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
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
        "endpoints": {
            "models": "/v1/models",
            "chat": "/v1/chat/completions",
            "healthz": "/healthz",
            "readyz": "/readyz",
        }
    }))
}

/// `GET /healthz` -> liveness probe (unauthenticated).
pub async fn healthz() -> &'static str {
    "ok"
}

/// `GET /readyz` -> readiness probe (unauthenticated).
///
/// Returns 200 when at least one backend is currently healthy; 503 otherwise.
/// Orchestrators should use this (not `/healthz`) to decide whether to send
/// traffic after deploy or when all upstreams are down.
pub async fn readyz(State(state): State<Arc<AppState>>) -> Response {
    if state.is_ready() {
        (axum::http::StatusCode::OK, "ok").into_response()
    } else {
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "not ready: no healthy backends",
        )
            .into_response()
    }
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
                // Only pin on 2xx so a 4xx/5xx does not stick the session to a
                // bad backend for the full session TTL.
                if resp.status().is_success() {
                    state.session_cache.insert(sid.clone(), idx);
                }
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
/// - `Err` only when no response was received at all (connect/timeout/capacity
///   failure), signaling the caller to retry on the next candidate.
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

    // Reserve capacity before contacting upstream so we never oversubscribe a
    // backend under concurrent load. Fail over if this backend is full.
    let guard =
        ConnGuard::try_acquire(&backend.active, backend.cfg.max_connections).ok_or_else(|| {
            format!(
                "backend '{}' at max_connections ({})",
                backend.cfg.name, backend.cfg.max_connections
            )
        })?;

    let upstream_started = Instant::now();
    // Header timeout only — long streaming bodies are bounded separately by
    // stream_idle_timeout_secs / stream_timeout_secs.
    let header_timeout = Duration::from_secs(backend.cfg.timeout_secs.max(1));

    let send_fut = http
        .request(method.clone(), &target)
        .headers(fwd)
        .body(send_body)
        .send();

    let upstream = match tokio::time::timeout(header_timeout, send_fut).await {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => return Err(format!("upstream request failed: {e}")),
        Err(_) => {
            return Err(format!(
                "upstream header timeout after {}s",
                header_timeout.as_secs()
            ))
        }
    };

    let status = upstream.status();
    let mut out_headers = upstream.headers().clone();
    strip_response_headers(&mut out_headers);

    let idle_timeout = if backend.cfg.stream_idle_timeout_secs > 0 {
        Some(Duration::from_secs(backend.cfg.stream_idle_timeout_secs))
    } else {
        None
    };
    let body_deadline = if backend.cfg.stream_timeout_secs > 0 {
        Some(upstream_started + Duration::from_secs(backend.cfg.stream_timeout_secs))
    } else {
        None
    };

    let mut upstream_stream = upstream.bytes_stream();
    let s = stream! {
        let _guard = guard; // held until the generator completes or is dropped
        let mut ttfb_recorded = false;
        let mut token_tracker = TokenTracker::new();

        loop {
            if let Some(deadline) = body_deadline {
                let now = Instant::now();
                if now >= deadline {
                    yield Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "upstream stream total timeout",
                    ));
                    break;
                }
            }

            let wait = match (idle_timeout, body_deadline) {
                (Some(idle), Some(deadline)) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        yield Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "upstream stream total timeout",
                        ));
                        break;
                    }
                    Some(idle.min(remaining))
                }
                (Some(idle), None) => Some(idle),
                (None, Some(deadline)) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        yield Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "upstream stream total timeout",
                        ));
                        break;
                    }
                    Some(remaining)
                }
                (None, None) => None,
            };

            let next = if let Some(limit) = wait {
                match tokio::time::timeout(limit, upstream_stream.next()).await {
                    Ok(item) => item,
                    Err(_) => {
                        let kind = if body_deadline.is_some_and(|d| Instant::now() >= d) {
                            "upstream stream total timeout"
                        } else {
                            "upstream stream idle timeout"
                        };
                        yield Err(std::io::Error::new(std::io::ErrorKind::TimedOut, kind));
                        break;
                    }
                }
            } else {
                upstream_stream.next().await
            };

            match next {
                Some(Ok(bytes)) => {
                    if !ttfb_recorded {
                        metrics::record_time_to_first_byte(upstream_started.elapsed());
                        ttfb_recorded = true;
                    }
                    token_tracker.observe_chunk(&bytes);
                    yield Ok::<_, std::io::Error>(bytes);
                }
                Some(Err(e)) => {
                    yield Err(std::io::Error::other(e.to_string()));
                    break;
                }
                None => break,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_acquire_respects_max_connections() {
        let active = Arc::new(AtomicU64::new(0));
        let g1 = ConnGuard::try_acquire(&active, 2).expect("first slot");
        let g2 = ConnGuard::try_acquire(&active, 2).expect("second slot");
        assert!(ConnGuard::try_acquire(&active, 2).is_none());
        assert_eq!(active.load(Ordering::Relaxed), 2);
        drop(g1);
        let g3 = ConnGuard::try_acquire(&active, 2).expect("slot after release");
        assert_eq!(active.load(Ordering::Relaxed), 2);
        drop(g2);
        drop(g3);
        assert_eq!(active.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn try_acquire_unlimited_when_max_zero() {
        let active = Arc::new(AtomicU64::new(0));
        let guards: Vec<_> = (0..50)
            .map(|_| ConnGuard::try_acquire(&active, 0).expect("unlimited"))
            .collect();
        assert_eq!(active.load(Ordering::Relaxed), 50);
        drop(guards);
        assert_eq!(active.load(Ordering::Relaxed), 0);
    }
}
