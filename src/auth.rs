use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// Extract a bearer token from an `Authorization: Bearer <key>` header.
pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let h = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let trimmed = h.trim();
    let rest = trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))?;
    let token = rest.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Paths exempt from all authentication (health/readiness probes, root).
fn is_public(path: &str) -> bool {
    path == "/healthz" || path == "/readyz" || path == "/"
}

/// Privileged operator endpoints gated by `server.admin_token` instead of
/// per-client `api_keys` (no model allowlists/concurrency accounting apply).
fn is_admin(path: &str) -> bool {
    path == "/metrics"
}

/// Constant-time byte comparison, to avoid leaking the admin token length/
/// prefix through response-time differences.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Axum middleware enforcing the OpenAI-style API key, plus a separate
/// `server.admin_token` bearer check for privileged endpoints (see
/// [`is_admin`]).
///
/// On success, attaches the resolved `Arc<ApiKeyInfo>` to the request's
/// extensions so downstream handlers (routing, model listing) can enforce
/// per-key model allowlists and concurrency limits without re-parsing the
/// header or re-checking the map.
pub async fn require_api_key(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();

    if is_public(path) {
        return next.run(request).await;
    }

    if is_admin(path) {
        return match &state.admin_token {
            None => next.run(request).await,
            Some(token) => match bearer_token(request.headers()) {
                Some(k) if constant_time_eq(&k, token) => next.run(request).await,
                _ => ApiError::unauthorized(
                    "missing or invalid admin token; expected 'Bearer <admin_token>'",
                )
                .into_response(),
            },
        };
    }

    if !state.auth_enabled {
        return next.run(request).await;
    }

    let headers = request.headers().clone();
    let key = match bearer_token(&headers) {
        Some(k) => k,
        None => {
            return ApiError::unauthorized(
                "missing Authorization header; expected 'Bearer <api_key>'",
            )
            .into_response()
        }
    };

    let info = match state.api_keys.get(&key) {
        Some(info) => info.clone(),
        None => return ApiError::unauthorized("invalid API key").into_response(),
    };
    request.extensions_mut().insert(info);

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_paths_bypass_all_auth() {
        for p in ["/healthz", "/readyz", "/"] {
            assert!(is_public(p), "{p} should be public");
            assert!(!is_admin(p), "{p} should not be admin-gated");
        }
    }

    #[test]
    fn metrics_is_admin_gated_not_public() {
        assert!(is_admin("/metrics"));
        assert!(!is_public("/metrics"));
    }

    #[test]
    fn client_routes_are_neither_public_nor_admin() {
        for p in ["/v1/models", "/models", "/v1/chat/completions"] {
            assert!(!is_public(p), "{p} should require client auth");
            assert!(!is_admin(p), "{p} should not be admin-gated");
        }
    }

    #[test]
    fn constant_time_eq_matches_string_equality() {
        assert!(constant_time_eq("secret-token", "secret-token"));
        assert!(!constant_time_eq("secret-token", "secret-tokeX"));
        assert!(!constant_time_eq("short", "longer-token"));
        assert!(!constant_time_eq("", "nonempty"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn bearer_token_parses_case_insensitive_prefix_and_trims() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer  abc123  ".try_into().unwrap(),
        );
        assert_eq!(bearer_token(&headers).as_deref(), Some("abc123"));

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "bearer abc123".try_into().unwrap(),
        );
        assert_eq!(bearer_token(&headers).as_deref(), Some("abc123"));
    }

    #[test]
    fn bearer_token_rejects_missing_or_malformed_header() {
        assert_eq!(bearer_token(&HeaderMap::new()), None);

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Basic abc123".try_into().unwrap(),
        );
        assert_eq!(bearer_token(&headers), None);

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer ".try_into().unwrap(),
        );
        assert_eq!(bearer_token(&headers), None);
    }
}
