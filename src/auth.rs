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

/// Paths exempt from authentication.
fn is_public(path: &str) -> bool {
    path == "/healthz" || path == "/readyz" || path == "/" || path == "/metrics"
}

/// Axum middleware enforcing the OpenAI-style API key.
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

    if !state.auth_enabled || is_public(path) {
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
