//! Web admin API / UI configuration.

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

fn default_bind() -> String {
    "127.0.0.1:8080".to_string()
}

fn default_requests_per_minute() -> u64 {
    60
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebConfig {
    #[serde(default = "default_true")]
    pub enable_api: bool,

    #[serde(default = "default_true")]
    pub enable_webui: bool,

    #[serde(default = "default_bind")]
    pub bind: String,

    pub api_key: Option<String>,

    #[serde(default)]
    pub cors_origins: Vec<String>,

    #[serde(default)]
    pub rate_limit: WebRateLimitConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebRateLimitConfig {
    #[serde(default = "default_requests_per_minute")]
    pub requests_per_minute: u64,
}

impl Default for WebRateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: default_requests_per_minute(),
        }
    }
}

const MIN_API_KEY_LENGTH: usize = 16;

impl WebConfig {
    fn bind_is_loopback(&self) -> bool {
        use std::net::{IpAddr, SocketAddr};

        if let Ok(addr) = self.bind.parse::<SocketAddr>() {
            return addr.ip().is_loopback();
        }
        let host = self
            .bind
            .rsplit_once(':')
            .map_or(self.bind.as_str(), |(h, _)| h)
            .trim_matches(['[', ']']);
        host.eq_ignore_ascii_case("localhost")
            || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
    }

    pub fn resolve_api_key(&mut self) -> Result<String, String> {
        match &self.api_key {
            Some(key) if key != "CHANGE-ME" && !key.is_empty() => {
                if key.len() < MIN_API_KEY_LENGTH {
                    return Err(format!(
                        "API key is too short ({} chars). Minimum length is {MIN_API_KEY_LENGTH} characters.",
                        key.len()
                    ));
                }
                Ok(key.clone())
            }
            _ if !self.bind_is_loopback() => Err(format!(
                "the web admin API is bound to a non-loopback address ({}) but no `api_key` is set. \
                 Set a strong `api_key` (>= {MIN_API_KEY_LENGTH} characters) in the [web] section, \
                 or bind to 127.0.0.1. Refusing to start an externally-reachable admin API without authentication.",
                self.bind
            )),
            _ => {
                let generated = uuid::Uuid::new_v4().to_string();
                tracing::warn!(
                    "No API key configured for loopback bind ({}) — generated an ephemeral key: {generated}",
                    self.bind
                );
                self.api_key = Some(generated.clone());
                Ok(generated)
            }
        }
    }
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            enable_api: true,
            enable_webui: true,
            bind: default_bind(),
            api_key: None,
            cors_origins: Vec::new(),
            rate_limit: WebRateLimitConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(bind: &str, api_key: Option<&str>) -> WebConfig {
        WebConfig {
            bind: bind.to_string(),
            api_key: api_key.map(str::to_string),
            ..WebConfig::default()
        }
    }

    #[test]
    fn loopback_without_key_autogenerates() {
        let mut cfg = config("127.0.0.1:8080", None);
        let key = cfg.resolve_api_key().expect("loopback may auto-generate");
        assert!(!key.is_empty());
        assert_eq!(cfg.api_key.as_deref(), Some(key.as_str()));
    }

    #[test]
    fn localhost_without_key_autogenerates() {
        let mut cfg = config("localhost:8080", None);
        assert!(cfg.resolve_api_key().is_ok());
    }

    #[test]
    fn non_loopback_without_key_is_hard_error() {
        for bind in ["0.0.0.0:8080", "192.168.1.10:8080", "[::]:8080"] {
            let mut cfg = config(bind, None);
            assert!(
                cfg.resolve_api_key().is_err(),
                "{bind} with no api_key must refuse to start"
            );
        }
    }

    #[test]
    fn non_loopback_with_placeholder_key_is_hard_error() {
        let mut cfg = config("0.0.0.0:8080", Some("CHANGE-ME"));
        assert!(cfg.resolve_api_key().is_err());
    }

    #[test]
    fn non_loopback_with_strong_key_is_accepted() {
        let mut cfg = config("0.0.0.0:8080", Some("a-sufficiently-long-api-key-value"));
        let key = cfg
            .resolve_api_key()
            .expect("strong key on any bind is fine");
        assert_eq!(key, "a-sufficiently-long-api-key-value");
    }
}
