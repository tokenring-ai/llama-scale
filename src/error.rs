use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// An error rendered as an OpenAI-style JSON error body.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    pub etype: &'static str,
}

impl ApiError {
    pub fn new(status: StatusCode, etype: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            etype,
            message: message.into(),
        }
    }

    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "invalid_api_key", msg)
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request_error", msg)
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found_error", msg)
    }

    pub fn service_unavailable(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "no_available_backend", msg)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({
            "error": {
                "message": self.message,
                "type": self.etype,
                "param": null,
                "code": null,
            }
        }));
        (self.status, body).into_response()
    }
}

/// Routing-policy errors (distinct from transport errors, which are retried).
#[derive(Debug, thiserror::Error)]
pub enum RouteError {
    #[error("request body is missing a JSON 'model' field; the router requires a model to route")]
    MissingModel,
    #[error("no healthy backend serves model '{0}'")]
    NoBackendForModel(String),
    #[error("all candidate backends for model '{0}' failed to respond")]
    AllBackendsFailed(String),
}

impl RouteError {
    pub fn into_api(self) -> ApiError {
        match self {
            RouteError::MissingModel => ApiError::bad_request(self.to_string()),
            RouteError::NoBackendForModel(_) => ApiError::service_unavailable(self.to_string()),
            RouteError::AllBackendsFailed(_) => ApiError::service_unavailable(self.to_string()),
        }
    }
}
