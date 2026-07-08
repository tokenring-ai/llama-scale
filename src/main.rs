mod access_log;
mod auth;
mod config;
mod error;
mod health;
mod models;
mod proxy;
mod routing;
mod state;

use anyhow::{Context, Result};
use axum::{
    extract::DefaultBodyLimit,
    middleware::{from_fn, from_fn_with_state},
    routing::get,
    Router,
};
use clap::Parser;
use std::path::PathBuf;
use tower_http::cors::CorsLayer;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "llama-scale",
    version,
    about = "OpenAI-compatible LLM router with session affinity and least-connections balancing"
)]
struct Args {
    /// Path to the YAML configuration file.
    #[arg(
        short,
        long,
        default_value = "config.yaml",
        env = "MODEL_ROUTER_CONFIG"
    )]
    config: PathBuf,
}

const MAX_REQUEST_BODY: usize = 100 * 1024 * 1024;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let cfg = config::Config::load(&args.config)?;
    let listen = cfg
        .server
        .listen
        .parse::<std::net::SocketAddr>()
        .map_err(|e| anyhow::anyhow!("invalid server.listen '{}': {e}", cfg.server.listen))?;

    // Initialize logging (console or file) before the first log call. The
    // returned guard must outlive all log emission, so it is held in main.
    let _log_guard = init_logging(&cfg.log)?;

    tracing::info!(
        backends = cfg.backends.len(),
        listen = %listen,
        log_destination = ?cfg.log.destination,
        "starting llama-scale"
    );

    let state = state::AppState::build(cfg)?;

    // Background control-plane tasks.
    let st = state.clone();
    tokio::spawn(async move {
        models::run_models_refresh(st).await;
    });
    let st = state.clone();
    tokio::spawn(async move {
        health::run_health_checks(st).await;
    });

    let app = Router::new()
        .route("/", get(proxy::root))
        .route("/healthz", get(proxy::healthz))
        .route("/v1/models", get(models::get_models))
        .route("/models", get(models::get_models))
        .fallback(proxy::proxy)
        .layer(from_fn_with_state(state.clone(), auth::require_api_key))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY))
        .layer(from_fn(access_log::access_log))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!("listening on {listen}");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Install the global tracing subscriber, writing either to stderr (console) or
/// to an appended log file via a non-blocking writer. The `RUST_LOG`
/// environment variable takes precedence over `log.level`.
fn init_logging(log: &config::LogConfig) -> Result<Option<WorkerGuard>> {
    let filter = EnvFilter::try_from_default_env().or_else(|_| {
        let lvl = log.level.as_deref().unwrap_or("info");
        EnvFilter::try_new(lvl)
    })?;

    match log.destination {
        config::LogDestination::Console => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_target(true)
                .init();
            Ok(None)
        }
        config::LogDestination::File => {
            let path = log.file.as_ref().ok_or_else(|| {
                anyhow::anyhow!("log.destination = 'file' requires a 'log.file' path")
            })?;
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating log directory {}", parent.display()))?;
                }
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .with_context(|| format!("opening log file {}", path.display()))?;
            let (writer, guard) = tracing_appender::non_blocking(file);
            tracing_subscriber::fmt()
                .with_writer(writer)
                .with_env_filter(filter)
                .with_target(true)
                .with_ansi(false)
                .init();
            Ok(Some(guard))
        }
    }
}
