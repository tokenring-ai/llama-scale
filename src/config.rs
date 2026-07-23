use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default = "default_models_refresh")]
    pub models_refresh_interval_secs: u64,
    #[serde(default = "default_health_interval")]
    pub health_check_interval_secs: u64,
    #[serde(default = "default_health_timeout")]
    pub health_check_timeout_secs: u64,
    #[serde(default = "default_session_ttl")]
    pub session_ttl_secs: u64,
    #[serde(default = "default_session_max")]
    pub session_max_entries: u64,
    #[serde(default)]
    pub backends: Vec<BackendConfig>,
}

fn default_models_refresh() -> u64 {
    30
}
fn default_health_interval() -> u64 {
    15
}
fn default_health_timeout() -> u64 {
    5
}
fn default_session_ttl() -> u64 {
    3600
}
fn default_session_max() -> u64 {
    100_000
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub listen: String,
    /// Bearer keys accepted from clients. Accepts either:
    /// - a flat list of key strings (legacy; every key is equivalent and
    ///   unrestricted), or
    /// - a map of `key -> { id, allowed_models, concurrent_requests }` for
    ///   multi-user setups where each key gets its own identity, model
    ///   allowlist, and concurrency cap.
    ///
    /// Leave empty to disable authentication (open access).
    #[serde(default)]
    pub api_keys: RawApiKeys,
    /// TLS termination for the listen socket. Omit to serve plain HTTP (e.g.
    /// behind a TLS-terminating reverse proxy).
    #[serde(default)]
    pub tls: Option<TlsConfig>,
    /// Bearer token guarding privileged endpoints (currently `/metrics`).
    /// Distinct from `api_keys`: it is not subject to model allowlists or
    /// concurrency caps and is meant for scrape/ops tooling, not clients.
    /// Leave unset to leave privileged endpoints open (not recommended for
    /// internet-facing deployments).
    #[serde(default)]
    pub admin_token: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TlsConfig {
    /// PEM-encoded certificate (or full chain) file path.
    pub cert_path: PathBuf,
    /// PEM-encoded private key file path.
    pub key_path: PathBuf,
}

/// Raw (as-parsed) shape of `server.api_keys`. Use [`ServerConfig::api_key_map`]
/// to get the normalized per-key view regardless of which form was used.
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum RawApiKeys {
    List(Vec<String>),
    Map(HashMap<String, RawApiKeyEntry>),
}

impl Default for RawApiKeys {
    fn default() -> Self {
        RawApiKeys::List(Vec::new())
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct RawApiKeyEntry {
    /// Opaque label identifying this key's owner in logs and diagnostics.
    /// Defaults to a truncated, non-secret prefix of the key when omitted.
    #[serde(default)]
    pub id: Option<String>,
    /// Model ids (or `prefix*` glob patterns) this key may request. Empty
    /// means unrestricted.
    #[serde(default)]
    pub allowed_models: Vec<String>,
    /// Max concurrent in-flight requests for this key. `0` (default) means
    /// unlimited.
    #[serde(default)]
    pub concurrent_requests: u64,
}

/// Normalized, post-validation view of one API key's settings.
#[derive(Debug, Clone)]
pub struct ApiKeyEntry {
    pub id: String,
    pub allowed_models: Vec<String>,
    pub concurrent_requests: u64,
}

/// Non-secret display id derived from a raw key when no explicit `id` is
/// configured, so logs never carry the full bearer token.
fn mask_key(key: &str) -> String {
    let prefix: String = key.chars().take(8).collect();
    format!("{prefix}…")
}

impl ServerConfig {
    /// Normalize `api_keys` (either form) into `key -> ApiKeyEntry`.
    pub fn api_key_map(&self) -> HashMap<String, ApiKeyEntry> {
        match &self.api_keys {
            RawApiKeys::List(keys) => keys
                .iter()
                .map(|k| {
                    (
                        k.clone(),
                        ApiKeyEntry {
                            id: mask_key(k),
                            allowed_models: Vec::new(),
                            concurrent_requests: 0,
                        },
                    )
                })
                .collect(),
            RawApiKeys::Map(entries) => entries
                .iter()
                .map(|(k, e)| {
                    (
                        k.clone(),
                        ApiKeyEntry {
                            id: e.id.clone().unwrap_or_else(|| mask_key(k)),
                            allowed_models: e.allowed_models.clone(),
                            concurrent_requests: e.concurrent_requests,
                        },
                    )
                })
                .collect(),
        }
    }
}

/// Does `model` match one of the `allowed` patterns? A pattern ending in `*`
/// matches by prefix (e.g. `gpt-4*` matches `gpt-4-turbo`); any other pattern
/// requires an exact match. An empty pattern list allows every model.
pub fn model_allowed(allowed: &[String], model: &str) -> bool {
    if allowed.is_empty() {
        return true;
    }
    allowed.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => model.starts_with(prefix),
        None => p == model,
    })
}

#[derive(Debug, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogDestination {
    #[default]
    Console,
    File,
    None,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct LogConfig {
    /// Where logs (app + access) are written.
    #[serde(default)]
    pub destination: LogDestination,
    /// File path appended to when `destination == file`.
    #[serde(default)]
    pub file: Option<PathBuf>,
    /// Optional fallback level (e.g. `info`, `debug`). Honored only when the
    /// `RUST_LOG` environment variable is not set.
    #[serde(default)]
    pub level: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BackendConfig {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub api_key: String,
    /// Max wait for upstream response *headers* (seconds). Does not bound the
    /// full stream body — see `stream_idle_timeout_secs` / `stream_timeout_secs`.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Max silence between body chunks while streaming (seconds). `0` disables
    /// the idle timeout. Defaults to 120.
    #[serde(default = "default_stream_idle_timeout")]
    pub stream_idle_timeout_secs: u64,
    /// Max total time for the response body after headers (seconds). `0` means
    /// unlimited (default), so long generations are not cut off.
    #[serde(default)]
    pub stream_timeout_secs: u64,
    /// Max concurrent in-flight proxied requests to this backend. `0` means
    /// unlimited (default). Saturated backends are skipped by the router.
    #[serde(default)]
    pub max_connections: u64,
    #[serde(default = "default_health_path")]
    pub health_path: String,
    /// Routing priority for new (unpinned) requests. Lower values are preferred;
    /// higher tiers are tried only when no healthy backend exists at a lower tier.
    #[serde(default)]
    pub fallback: u32,
    #[serde(default)]
    pub model_aliases: Vec<ModelAlias>,
}

fn default_timeout() -> u64 {
    120
}
fn default_stream_idle_timeout() -> u64 {
    120
}
fn default_health_path() -> String {
    "/v1/models".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelAlias {
    pub alias: String,
    pub real: String,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        // Parse first, then expand ${ENV} only inside real string *values*.
        // This avoids matching placeholders that appear in comments (comments
        // are stripped during parsing).
        let mut value: serde_yaml::Value =
            serde_yaml::from_str(&raw).context("parsing config YAML")?;
        expand_env_in_value(&mut value)?;
        let cfg: Config = serde_yaml::from_value(value).context("deserializing config")?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        if self.backends.is_empty() {
            return Err(anyhow!("config must define at least one backend"));
        }

        self.server
            .listen
            .parse::<std::net::SocketAddr>()
            .map_err(|e| {
                anyhow!(
                    "server.listen '{}' is not a valid socket address: {e}",
                    self.server.listen
                )
            })?;

        if let Some(tls) = &self.server.tls {
            if !tls.cert_path.is_file() {
                return Err(anyhow!(
                    "server.tls.cert_path '{}' does not exist or is not a file",
                    tls.cert_path.display()
                ));
            }
            if !tls.key_path.is_file() {
                return Err(anyhow!(
                    "server.tls.key_path '{}' does not exist or is not a file",
                    tls.key_path.display()
                ));
            }
        }

        if let Some(token) = &self.server.admin_token {
            if token.trim().is_empty() {
                return Err(anyhow!("server.admin_token must not be empty"));
            }
        }

        if self.log.destination == LogDestination::File
            && self
                .log
                .file
                .as_ref()
                .map(|p| p.as_os_str().is_empty())
                .unwrap_or(true)
        {
            return Err(anyhow!(
                "log.destination = 'file' requires a 'log.file' path"
            ));
        }

        let mut names = HashSet::new();
        for b in &self.backends {
            if b.name.trim().is_empty() {
                return Err(anyhow!("each backend requires a non-empty 'name'"));
            }
            if !names.insert(b.name.clone()) {
                return Err(anyhow!("duplicate backend name '{}'", b.name));
            }

            let url = b
                .url
                .parse::<url::Url>()
                .map_err(|e| anyhow!("backend '{}' has invalid url '{}': {e}", b.name, b.url))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(anyhow!(
                    "backend '{}' url must use http or https scheme",
                    b.name
                ));
            }

            if b.health_path.is_empty() || !b.health_path.starts_with('/') {
                return Err(anyhow!(
                    "backend '{}' health_path must start with '/'",
                    b.name
                ));
            }

            let mut aliases = HashSet::new();
            for a in &b.model_aliases {
                if a.alias.trim().is_empty() || a.real.trim().is_empty() {
                    return Err(anyhow!(
                        "backend '{}' has a model_alias with empty 'alias' or 'real'",
                        b.name
                    ));
                }
                if !aliases.insert(a.alias.clone()) {
                    return Err(anyhow!(
                        "backend '{}' has duplicate model alias '{}'",
                        b.name,
                        a.alias
                    ));
                }
            }
        }

        for (key, entry) in self.server.api_key_map() {
            if key.trim().is_empty() {
                return Err(anyhow!("server.api_keys contains an empty key"));
            }
            for m in &entry.allowed_models {
                if m.trim().is_empty() {
                    return Err(anyhow!(
                        "api key '{}' has an empty entry in allowed_models",
                        entry.id
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Expand `${VAR}` references against the process environment.
/// Missing variables are a hard error to avoid silently routing with empty keys.
fn expand_env(input: &str) -> Result<String> {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && matches!(chars.peek(), Some('{')) {
            chars.next();
            let mut name = String::new();
            let mut closed = false;
            for nc in chars.by_ref() {
                if nc == '}' {
                    closed = true;
                    break;
                }
                name.push(nc);
            }
            if !closed {
                return Err(anyhow!("unterminated '${{' in config value"));
            }
            let trimmed = name.trim();
            let val = std::env::var(trimmed).map_err(|_| {
                anyhow!("environment variable '{trimmed}' referenced by config is not set")
            })?;
            out.push_str(&val);
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

/// Recursively expand `${ENV}` inside every string value of a YAML tree.
/// Comments and mapping keys are left untouched (comments are already stripped
/// by the time the YAML is parsed into this tree).
fn expand_env_in_value(value: &mut serde_yaml::Value) -> Result<()> {
    match value {
        serde_yaml::Value::Mapping(m) => {
            for (_, v) in m.iter_mut() {
                expand_env_in_value(v)?;
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            for v in seq {
                expand_env_in_value(v)?;
            }
        }
        serde_yaml::Value::String(s) => {
            *s = expand_env(s)?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_env_vars() {
        std::env::set_var("MR_TEST_KEY", "secretvalue");
        let out = expand_env("key: ${MR_TEST_KEY}").unwrap();
        assert_eq!(out, "key: secretvalue");
    }

    #[test]
    fn missing_env_is_error() {
        let res = expand_env("key: ${MR_DEFINITELY_MISSING_VAR_X}");
        assert!(res.is_err());
    }

    #[test]
    fn unterminated_is_error() {
        let res = expand_env("key: ${OPEN");
        assert!(res.is_err());
    }

    #[test]
    fn parses_minimal_config() {
        let yaml = r#"
server:
  listen: 127.0.0.1:8080
  api_keys: [sk-test]
backends:
  - name: a
    url: https://api.openai.com/v1
    api_key: k
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.models_refresh_interval_secs, 30);
        assert_eq!(cfg.health_check_interval_secs, 15);
        assert!(cfg.backends[0].model_aliases.is_empty());
        assert_eq!(cfg.backends[0].timeout_secs, 120);
        assert_eq!(cfg.backends[0].stream_idle_timeout_secs, 120);
        assert_eq!(cfg.backends[0].stream_timeout_secs, 0);
        assert_eq!(cfg.backends[0].max_connections, 0);

        let keys = cfg.server.api_key_map();
        let entry = keys.get("sk-test").unwrap();
        assert_eq!(entry.id, "sk-test…"); // legacy list form: id defaults to a masked key
        assert!(entry.allowed_models.is_empty());
        assert_eq!(entry.concurrent_requests, 0);
    }

    #[test]
    fn parses_multi_user_api_keys_map() {
        let yaml = r#"
server:
  listen: 127.0.0.1:8080
  api_keys:
    sk-alice:
      id: alice
      allowed_models: [gpt-4, llama-3*]
      concurrent_requests: 2
    sk-bob:
      id: bob
backends:
  - name: a
    url: https://api.openai.com/v1
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        cfg.validate().unwrap();
        let keys = cfg.server.api_key_map();

        let alice = keys.get("sk-alice").unwrap();
        assert_eq!(alice.id, "alice");
        assert_eq!(alice.allowed_models, vec!["gpt-4", "llama-3*"]);
        assert_eq!(alice.concurrent_requests, 2);

        // bob has no allowed_models / concurrent_requests -> unrestricted defaults.
        let bob = keys.get("sk-bob").unwrap();
        assert_eq!(bob.id, "bob");
        assert!(bob.allowed_models.is_empty());
        assert_eq!(bob.concurrent_requests, 0);
    }

    #[test]
    fn empty_allowed_models_entry_is_rejected() {
        let yaml = r#"
server:
  listen: 127.0.0.1:8080
  api_keys:
    sk-alice:
      allowed_models: [""]
backends:
  - name: a
    url: https://api.openai.com/v1
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn model_allowed_matches_exact_and_wildcard() {
        let patterns = vec!["gpt-4".to_string(), "llama-3*".to_string()];
        assert!(model_allowed(&patterns, "gpt-4"));
        assert!(model_allowed(&patterns, "llama-3-70b"));
        assert!(!model_allowed(&patterns, "gpt-4-turbo"));
        assert!(!model_allowed(&patterns, "claude-3"));
        // No patterns -> unrestricted.
        assert!(model_allowed(&[], "anything"));
    }

    #[test]
    fn parses_concurrency_and_stream_timeouts() {
        let yaml = r#"
server:
  listen: 127.0.0.1:8080
backends:
  - name: a
    url: http://127.0.0.1:11434/v1
    timeout_secs: 30
    stream_idle_timeout_secs: 60
    stream_timeout_secs: 600
    max_connections: 4
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        cfg.validate().unwrap();
        let b = &cfg.backends[0];
        assert_eq!(b.timeout_secs, 30);
        assert_eq!(b.stream_idle_timeout_secs, 60);
        assert_eq!(b.stream_timeout_secs, 600);
        assert_eq!(b.max_connections, 4);
    }

    #[test]
    fn env_expansion_ignores_placeholders_in_comments() {
        // The literal `${VAR}` in the comment must NOT be treated as an
        // environment reference (regression for the raw-text scanner bug).
        let yaml = r#"
# reference secrets as ${VAR}
server:
  listen: 127.0.0.1:8080
  api_keys: [sk-test]
backends:
  - name: a
    url: https://api.openai.com/v1
"#;
        let mut value: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        expand_env_in_value(&mut value).unwrap();
        let cfg: Config = serde_yaml::from_value(value).unwrap();
        cfg.validate().unwrap();
    }

    #[test]
    fn env_expansion_applies_to_values_only() {
        std::env::set_var("MR_TEST_BACKEND_KEY", "supersecret");
        let yaml = r#"
server:
  listen: 127.0.0.1:8080
backends:
  - name: a
    url: https://api.openai.com/v1
    api_key: ${MR_TEST_BACKEND_KEY}
"#;
        let mut value: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        expand_env_in_value(&mut value).unwrap();
        let cfg: Config = serde_yaml::from_value(value).unwrap();
        assert_eq!(cfg.backends[0].api_key, "supersecret");
    }

    #[test]
    fn tls_requires_existing_cert_and_key_files() {
        let dir = std::env::temp_dir().join(format!("mr-tls-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cert = dir.join("cert.pem");
        let key = dir.join("key.pem");
        std::fs::write(&cert, "cert").unwrap();
        std::fs::write(&key, "key").unwrap();

        let yaml = format!(
            r#"
server:
  listen: 127.0.0.1:8080
  tls:
    cert_path: {}
    key_path: {}
backends:
  - name: a
    url: https://api.openai.com/v1
"#,
            cert.display(),
            key.display()
        );
        let cfg: Config = serde_yaml::from_str(&yaml).unwrap();
        cfg.validate().unwrap();

        let missing_yaml = format!(
            r#"
server:
  listen: 127.0.0.1:8080
  tls:
    cert_path: {}
    key_path: {}/does-not-exist.pem
backends:
  - name: a
    url: https://api.openai.com/v1
"#,
            cert.display(),
            dir.display()
        );
        let cfg: Config = serde_yaml::from_str(&missing_yaml).unwrap();
        assert!(cfg.validate().is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn admin_token_defaults_to_unset() {
        let yaml = r#"
server:
  listen: 127.0.0.1:8080
backends:
  - name: a
    url: https://api.openai.com/v1
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.server.admin_token.is_none());
    }

    #[test]
    fn empty_admin_token_is_rejected() {
        let yaml = r#"
server:
  listen: 127.0.0.1:8080
  admin_token: ""
backends:
  - name: a
    url: https://api.openai.com/v1
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn admin_token_expands_from_env() {
        std::env::set_var("MR_TEST_ADMIN_TOKEN", "supersecret-admin");
        let yaml = r#"
server:
  listen: 127.0.0.1:8080
  admin_token: ${MR_TEST_ADMIN_TOKEN}
backends:
  - name: a
    url: https://api.openai.com/v1
"#;
        let mut value: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        expand_env_in_value(&mut value).unwrap();
        let cfg: Config = serde_yaml::from_value(value).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.server.admin_token.as_deref(), Some("supersecret-admin"));
    }
}
