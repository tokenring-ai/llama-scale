use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::HashSet;
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
    #[serde(default)]
    pub api_keys: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogDestination {
    #[default]
    Console,
    File,
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
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_health_path")]
    pub health_path: String,
    #[serde(default)]
    pub model_aliases: Vec<ModelAlias>,
}

fn default_timeout() -> u64 {
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
}
