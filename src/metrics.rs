use crate::state::AppState;
use axum::extract::{Extension, State};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

static AVG_TOKENS_PER_SECOND_BITS: AtomicU64 = AtomicU64::new(0);

const TPS_EMA_ALPHA: f64 = 0.2;

const REQUEST_DURATION_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0,
];

const TTFB_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
];

/// Install the global Prometheus recorder and return a handle for rendering.
pub fn init() -> PrometheusHandle {
    let builder = PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("llama_scale_request_duration_seconds".into()),
            REQUEST_DURATION_BUCKETS,
        )
        .expect("invalid request duration histogram buckets")
        .set_buckets_for_metric(
            Matcher::Full("llama_scale_time_to_first_byte_seconds".into()),
            TTFB_BUCKETS,
        )
        .expect("invalid time-to-first-byte histogram buckets");

    let handle = builder
        .install_recorder()
        .expect("failed to install prometheus metrics recorder");

    metrics::describe_gauge!(
        "llama_scale_active_connections",
        "In-flight proxied requests per backend (held for the full upstream response body)"
    );
    metrics::describe_counter!(
        "llama_scale_requests_total",
        "HTTP requests handled by outcome"
    );
    metrics::describe_histogram!(
        "llama_scale_request_duration_seconds",
        "End-to-end HTTP request duration in seconds"
    );
    metrics::describe_histogram!(
        "llama_scale_time_to_first_byte_seconds",
        "Time from upstream request start until the first response byte is received"
    );
    metrics::describe_counter!(
        "llama_scale_tokens_generated_total",
        "Completion/output tokens observed in proxied LLM responses"
    );
    metrics::describe_gauge!(
        "llama_scale_tokens_per_second_avg",
        "Exponential moving average of completion tokens per second across finished streams"
    );

    handle
}

/// `GET /metrics` — Prometheus scrape endpoint. Gated by `server.admin_token`
/// (see `auth::require_api_key`) when configured.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Extension(handle): Extension<PrometheusHandle>,
) -> String {
    for backend in &state.backends {
        let active = backend.active_count();
        metrics::gauge!(
            "llama_scale_active_connections",
            "backend" => backend.cfg.name.clone()
        )
        .set(active as f64);
    }

    let bits = AVG_TOKENS_PER_SECOND_BITS.load(Ordering::Relaxed);
    if bits != 0 {
        let avg = f64::from_bits(bits);
        if avg.is_finite() {
            metrics::gauge!("llama_scale_tokens_per_second_avg").set(avg);
        }
    }

    handle.render()
}

/// Classify a response status into a request-outcome counter label.
pub fn record_request_outcome(status: u16) {
    let outcome = if (200..300).contains(&status) {
        "success"
    } else if status == 401 {
        "auth_failure"
    } else if (500..600).contains(&status) {
        "server_error"
    } else {
        return;
    };

    metrics::counter!("llama_scale_requests_total", "outcome" => outcome).increment(1);
}

pub fn record_request_duration(duration: Duration) {
    metrics::histogram!("llama_scale_request_duration_seconds").record(duration.as_secs_f64());
}

pub fn record_time_to_first_byte(ttfb: Duration) {
    metrics::histogram!("llama_scale_time_to_first_byte_seconds").record(ttfb.as_secs_f64());
}

/// Record token throughput for a finished upstream response stream.
pub fn record_token_generation(tokens: u64, stream_duration: Duration) {
    if tokens == 0 {
        return;
    }

    metrics::counter!("llama_scale_tokens_generated_total").increment(tokens);

    let secs = stream_duration.as_secs_f64();
    if secs <= 0.0 {
        return;
    }

    let instant_tps = tokens as f64 / secs;
    let prev_bits = AVG_TOKENS_PER_SECOND_BITS.load(Ordering::Relaxed);
    let prev = if prev_bits == 0 {
        instant_tps
    } else {
        f64::from_bits(prev_bits)
    };
    let next = TPS_EMA_ALPHA * instant_tps + (1.0 - TPS_EMA_ALPHA) * prev;
    AVG_TOKENS_PER_SECOND_BITS.store(next.to_bits(), Ordering::Relaxed);
    metrics::gauge!("llama_scale_tokens_per_second_avg").set(next);
}

/// Accumulates token counts while proxying an upstream response body.
pub struct TokenTracker {
    line_buf: String,
    accumulated: u64,
    final_usage: Option<u64>,
    stream_started: Option<Instant>,
}

impl TokenTracker {
    pub fn new() -> Self {
        Self {
            line_buf: String::new(),
            accumulated: 0,
            final_usage: None,
            stream_started: None,
        }
    }

    pub fn observe_chunk(&mut self, bytes: &[u8]) {
        let chunk = String::from_utf8_lossy(bytes);
        self.line_buf.push_str(&chunk);

        while let Some(pos) = self.line_buf.find('\n') {
            let line: String = self.line_buf.drain(..=pos).collect();
            self.observe_line(line.trim());
        }
    }

    fn observe_line(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }

        let payload = line
            .strip_prefix("data: ")
            .or_else(|| line.strip_prefix("data:"))
            .unwrap_or(line)
            .trim();

        if payload == "[DONE]" {
            return;
        }

        let Ok(value) = serde_json::from_str::<Value>(payload) else {
            return;
        };

        if let Some(tokens) = usage_completion_tokens(&value) {
            self.final_usage = Some(tokens);
            return;
        }

        if let Some(delta) = delta_token_estimate(&value) {
            if self.stream_started.is_none() {
                self.stream_started = Some(Instant::now());
            }
            self.accumulated += delta;
        }
    }

    pub fn total_tokens(&self) -> u64 {
        self.final_usage.unwrap_or(self.accumulated)
    }

    pub fn stream_duration(&self) -> Option<Duration> {
        self.stream_started.map(|start| start.elapsed())
    }

    pub fn finish(&mut self) {
        if self.line_buf.trim().is_empty() {
            return;
        }
        let remaining = std::mem::take(&mut self.line_buf);
        self.observe_line(remaining.trim());
    }
}

fn usage_completion_tokens(value: &Value) -> Option<u64> {
    value
        .get("usage")
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|t| t.as_u64())
}

fn delta_token_estimate(value: &Value) -> Option<u64> {
    let choices = value.get("choices")?.as_array()?;
    for choice in choices {
        if let Some(content) = choice
            .get("delta")
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
        {
            return Some(approximate_tokens(content));
        }
        if let Some(content) = choice
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
        {
            return Some(approximate_tokens(content));
        }
        if let Some(text) = choice.get("text").and_then(|t| t.as_str()) {
            return Some(approximate_tokens(text));
        }
    }
    None
}

fn approximate_tokens(text: &str) -> u64 {
    if text.is_empty() {
        0
    } else {
        ((text.len() as u64) + 3) / 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_generation_updates_ema() {
        AVG_TOKENS_PER_SECOND_BITS.store(0, Ordering::Relaxed);
        record_token_generation(100, Duration::from_secs(10));
        let first = f64::from_bits(AVG_TOKENS_PER_SECOND_BITS.load(Ordering::Relaxed));
        assert!((first - 10.0).abs() < f64::EPSILON);

        record_token_generation(50, Duration::from_secs(5));
        let second = f64::from_bits(AVG_TOKENS_PER_SECOND_BITS.load(Ordering::Relaxed));
        assert!((second - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parses_streaming_and_usage_tokens() {
        let mut tracker = TokenTracker::new();
        tracker.observe_chunk(b"data: {\"choices\":[{\"delta\":{\"content\":\"hello world\"}}]}\n");
        assert_eq!(tracker.total_tokens(), 3);

        tracker
            .observe_chunk(b"data: {\"usage\":{\"completion_tokens\":42,\"prompt_tokens\":10}}\n");
        assert_eq!(tracker.total_tokens(), 42);
    }
}
