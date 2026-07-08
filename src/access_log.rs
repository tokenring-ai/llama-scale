use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use std::time::Instant;

/// Extension inserted by the proxy onto successful responses so the access-log
/// middleware can record which backend served the request.
#[derive(Clone, Debug)]
pub struct RoutedBackend(pub String);

/// Access-log middleware: emits one structured line per request with method,
/// path, status, the backend it was routed to (when applicable), and latency.
pub async fn access_log(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let start = Instant::now();

    let response = next.run(request).await;

    let latency = start.elapsed();
    let backend = response
        .extensions()
        .get::<RoutedBackend>()
        .map(|b| b.0.as_str())
        .unwrap_or("-");

    tracing::info!(
        method = %method,
        path = %path,
        status = response.status().as_u16(),
        backend = %backend,
        latency_ms = latency.as_millis() as u64,
        "request"
    );

    response
}
