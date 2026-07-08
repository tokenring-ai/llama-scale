use crate::state::AppState;
use std::sync::Arc;

/// Background loop performing periodic health probes against every backend.
///
/// This is the single authority on each backend's `healthy` flag. A probe is a
/// `GET {health_url}` returning 2xx; anything else marks the backend down so
/// candidate selection skips it.
pub async fn run_health_checks(state: Arc<AppState>) {
    loop {
        for backend in &state.backends {
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
        tokio::time::sleep(state.health_check_interval).await;
    }
}
