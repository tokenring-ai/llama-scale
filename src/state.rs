use crate::config::Config;
use anyhow::{anyhow, Result};
use arc_swap::ArcSwap;
use moka::sync::Cache;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Runtime view of a configured backend.
pub struct Backend {
    pub cfg: crate::config::BackendConfig,
    /// Base URL exactly as configured (validated, trailing slashes removed).
    pub base_url: String,
    /// `alias -> real` map. Empty when no aliases are configured.
    pub aliases: HashMap<String, String>,
    pub has_aliases: bool,
    pub healthy: Arc<AtomicBool>,
    /// Number of in-flight requests being proxied to this backend.
    pub active: Arc<AtomicU64>,
    /// Raw upstream model ids (from `GET {base_url}/models`). Empty until first
    /// successful refresh. Only consulted when `has_aliases` is false.
    pub raw_models: Arc<RwLock<Vec<String>>>,
}

impl Backend {
    pub fn from_cfg(cfg: crate::config::BackendConfig) -> Result<Self> {
        let parsed = cfg
            .url
            .parse::<url::Url>()
            .map_err(|e| anyhow!("invalid url for backend '{}': {e}", cfg.name))?;
        let mut base = parsed.to_string();
        while base.ends_with('/') {
            base.pop();
        }

        let aliases: HashMap<String, String> = cfg
            .model_aliases
            .iter()
            .map(|a| (a.alias.clone(), a.real.clone()))
            .collect();
        let has_aliases = !aliases.is_empty();

        Ok(Self {
            cfg,
            base_url: base,
            aliases,
            has_aliases,
            // Start unhealthy until the first successful health probe so we do
            // not route traffic to unverified backends during cold start.
            healthy: Arc::new(AtomicBool::new(false)),
            active: Arc::new(AtomicU64::new(0)),
            raw_models: Arc::new(RwLock::new(Vec::new())),
        })
    }

    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }

    /// Mark this backend up/down. Used by the health-check task (sole authority).
    pub fn set_healthy(&self, up: bool) {
        self.healthy.store(up, Ordering::Relaxed);
    }

    pub fn active_count(&self) -> u64 {
        self.active.load(Ordering::Relaxed)
    }

    /// True when `max_connections` is set and the in-flight count is at/above it.
    /// `max_connections == 0` means unlimited.
    pub fn is_saturated(&self) -> bool {
        let max = self.cfg.max_connections;
        max > 0 && self.active_count() >= max
    }

    /// Does this backend serve the requested model name?
    /// - With aliases configured: only the alias names are served.
    /// - Without aliases: any id reported by the upstream `/models` cache.
    pub fn serves(&self, model: &str) -> bool {
        if self.has_aliases {
            self.aliases.contains_key(model)
        } else {
            self.raw_models
                .read()
                .expect("raw_models poisoned")
                .iter()
                .any(|m| m == model)
        }
    }

    /// The list of model names this backend contributes to the merged `/models`
    /// view: alias names if configured, otherwise the cached upstream ids.
    pub fn display_models(&self) -> Vec<String> {
        if self.has_aliases {
            self.aliases.keys().cloned().collect()
        } else {
            self.raw_models.read().expect("raw_models poisoned").clone()
        }
    }

    /// Build the health-probe URL. `health_path` is treated as host-root
    /// absolute and replaces the path of the configured base URL, so
    /// `/health` resolves to `http://host:port/health` (Ollama) and the default
    /// `/v1/models` resolves to `http://host:port/v1/models`.
    pub fn health_url(&self) -> String {
        let path = if self.cfg.health_path.is_empty() {
            "/v1/models"
        } else {
            self.cfg.health_path.as_str()
        };
        match self.base_url.parse::<url::Url>() {
            Ok(mut u) => {
                u.set_path(path);
                u.set_query(None);
                u.to_string()
            }
            Err(_) => format!("{}{path}", self.base_url),
        }
    }
}

/// A single entry in the merged `/models` listing.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub owned_by: String,
}

/// Shared application state.
pub struct AppState {
    pub backends: Vec<Arc<Backend>>,
    pub api_keys: Arc<HashSet<String>>,
    pub auth_enabled: bool,
    /// `session_id -> backend index` affinity cache (TTL + capacity bounded).
    pub session_cache: Cache<String, usize>,
    /// Merged, deduplicated model list served by `/v1/models` and `/models`.
    pub models_list: ArcSwap<Vec<ModelInfo>>,
    pub http: reqwest::Client,
    pub models_refresh_interval: Duration,
    pub health_check_interval: Duration,
    pub health_check_timeout: Duration,
}

impl AppState {
    pub fn build(cfg: Config) -> Result<Arc<Self>> {
        if cfg.backends.is_empty() {
            return Err(anyhow!("no backends configured"));
        }

        let mut backends = Vec::with_capacity(cfg.backends.len());
        for bc in cfg.backends {
            backends.push(Arc::new(Backend::from_cfg(bc)?));
        }

        let api_keys: HashSet<String> = cfg.server.api_keys.iter().cloned().collect();
        let auth_enabled = !api_keys.is_empty();
        if !auth_enabled {
            tracing::warn!(
                "no server.api_keys configured -> authentication is DISABLED (open access)"
            );
        }

        let session_cache = Cache::builder()
            .time_to_live(Duration::from_secs(cfg.session_ttl_secs.max(1)))
            .max_capacity(cfg.session_max_entries.max(1))
            .build();

        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60))
            .build()?;

        let state = Arc::new(Self {
            backends,
            api_keys: Arc::new(api_keys),
            auth_enabled,
            session_cache,
            models_list: ArcSwap::from_pointee(Vec::new()),
            http,
            models_refresh_interval: Duration::from_secs(cfg.models_refresh_interval_secs.max(1)),
            health_check_interval: Duration::from_secs(cfg.health_check_interval_secs.max(1)),
            health_check_timeout: Duration::from_secs(cfg.health_check_timeout_secs.max(1)),
        });

        Ok(state)
    }

    pub fn backend(&self, idx: usize) -> Arc<Backend> {
        self.backends[idx].clone()
    }

    /// Ready to receive traffic when at least one backend is currently healthy.
    pub fn is_ready(&self) -> bool {
        self.backends.iter().any(|b| b.is_healthy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BackendConfig;
    use std::sync::atomic::Ordering;

    fn backend(base: &str, health_path: &str) -> Backend {
        let cfg = BackendConfig {
            name: "b".into(),
            url: base.into(),
            api_key: String::new(),
            timeout_secs: 30,
            stream_idle_timeout_secs: 120,
            stream_timeout_secs: 0,
            max_connections: 0,
            health_path: health_path.into(),
            fallback: 0,
            model_aliases: vec![],
        };
        Backend::from_cfg(cfg).unwrap()
    }

    #[test]
    fn health_path_resolves_root_relative() {
        // `/health` replaces the path -> hits the host root, not /v1/health.
        let b = backend("http://192.168.15.25:11434/v1", "/health");
        assert_eq!(b.health_url(), "http://192.168.15.25:11434/health");

        // default `/v1/models` lands on the versioned endpoint.
        let b = backend("https://api.openai.com/v1", "/v1/models");
        assert_eq!(b.health_url(), "https://api.openai.com/v1/models");

        // trailing slash on base is normalized away first.
        let b = backend("http://localhost:11434/v1/", "/health");
        assert_eq!(b.health_url(), "http://localhost:11434/health");
    }

    #[test]
    fn base_url_trims_trailing_slash() {
        let b = backend("http://localhost:11434/v1/", "/models");
        assert_eq!(b.base_url, "http://localhost:11434/v1");
    }

    #[test]
    fn backends_start_unhealthy() {
        let b = backend("http://localhost:11434/v1", "/health");
        assert!(!b.is_healthy());
    }

    #[test]
    fn is_ready_requires_at_least_one_healthy_backend() {
        let cfg = Config {
            server: crate::config::ServerConfig {
                listen: "127.0.0.1:0".into(),
                api_keys: vec![],
            },
            log: Default::default(),
            models_refresh_interval_secs: 30,
            health_check_interval_secs: 15,
            health_check_timeout_secs: 5,
            session_ttl_secs: 3600,
            session_max_entries: 1000,
            backends: vec![BackendConfig {
                name: "a".into(),
                url: "http://127.0.0.1:9/v1".into(),
                api_key: String::new(),
                timeout_secs: 30,
                stream_idle_timeout_secs: 120,
                stream_timeout_secs: 0,
                max_connections: 0,
                health_path: "/health".into(),
                fallback: 0,
                model_aliases: vec![],
            }],
        };
        let state = AppState::build(cfg).unwrap();
        assert!(!state.is_ready());
        state.backends[0].set_healthy(true);
        assert!(state.is_ready());
    }

    #[test]
    fn is_saturated_respects_max_connections() {
        let cfg = BackendConfig {
            name: "b".into(),
            url: "http://localhost:1/v1".into(),
            api_key: String::new(),
            timeout_secs: 30,
            stream_idle_timeout_secs: 120,
            stream_timeout_secs: 0,
            max_connections: 2,
            health_path: "/health".into(),
            fallback: 0,
            model_aliases: vec![],
        };
        let b = Backend::from_cfg(cfg).unwrap();
        assert!(!b.is_saturated());
        b.active.store(1, Ordering::Relaxed);
        assert!(!b.is_saturated());
        b.active.store(2, Ordering::Relaxed);
        assert!(b.is_saturated());
    }
}
