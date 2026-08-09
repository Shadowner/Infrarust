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

    /// `None` follows [`enable_api`](Self::enable_api): the dashboard is
    /// served by the API, so turning the API off cannot leave the UI on by
    /// default. Read it through [`webui_enabled`](Self::webui_enabled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_webui: Option<bool>,

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

/// What the configured `api_key` amounts to at startup.
enum ApiKey {
    /// Usable as it stands.
    Usable(String),
    /// Nothing usable is configured; a loopback bind may mint an ephemeral key.
    Missing,
    /// The redaction placeholder, standing in for a key this config does not
    /// carry.
    Redacted,
    /// Configured but unusable, with the reason.
    Invalid(String),
}

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

    /// Whether the dashboard is served. It cannot run without the API.
    pub fn webui_enabled(&self) -> bool {
        self.enable_webui.unwrap_or(self.enable_api)
    }

    fn classify_api_key(&self) -> ApiKey {
        match self.api_key.as_deref() {
            Some(key) if key == crate::secrets::REDACTED => ApiKey::Redacted,
            Some(key) if key != "CHANGE-ME" && !key.is_empty() => {
                if key.len() < MIN_API_KEY_LENGTH {
                    return ApiKey::Invalid(format!(
                        "API key is too short ({} chars). Minimum length is {MIN_API_KEY_LENGTH} characters.",
                        key.len()
                    ));
                }
                ApiKey::Usable(key.to_string())
            }
            _ if !self.bind_is_loopback() => ApiKey::Invalid(format!(
                "the web admin API is bound to a non-loopback address ({}) but no `api_key` is set. \
                 Set a strong `api_key` (>= {MIN_API_KEY_LENGTH} characters) in the [web] section, \
                 or bind to 127.0.0.1. Refusing to start an externally-reachable admin API without authentication.",
                self.bind
            )),
            _ => ApiKey::Missing,
        }
    }

    /// Whether [`resolve_api_key`](Self::resolve_api_key) would succeed, so a
    /// configuration that cannot boot is refused before it is written.
    ///
    /// A redacted key passes: it stands for a stored key this config does not
    /// carry, and a write puts the real one back.
    ///
    /// # Errors
    ///
    /// Returns the message `resolve_api_key` would fail with.
    pub fn check_api_key(&self) -> Result<(), String> {
        match self.classify_api_key() {
            ApiKey::Usable(_) | ApiKey::Missing | ApiKey::Redacted => Ok(()),
            ApiKey::Invalid(reason) => Err(reason),
        }
    }

    /// The key the admin API authenticates with, minting an ephemeral one for
    /// a loopback bind that configures none.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when the configured key cannot be used.
    pub fn resolve_api_key(&mut self) -> Result<String, String> {
        match self.classify_api_key() {
            ApiKey::Usable(key) => Ok(key),
            ApiKey::Invalid(reason) => Err(reason),
            ApiKey::Redacted => Err(format!(
                "`api_key` in the [web] section is the redaction placeholder '{}'. \
                 It is not a key: put the real one back.",
                crate::secrets::REDACTED
            )),
            ApiKey::Missing => {
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
            enable_webui: None,
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

    /// Every startup failure must be visible to a write, or the API can store
    /// a config the proxy then refuses to boot with.
    #[test]
    fn check_agrees_with_resolve_on_every_key() {
        let keys = [
            None,
            Some(""),
            Some("CHANGE-ME"),
            Some("hunter2"),
            Some(crate::secrets::REDACTED),
            Some("a-sufficiently-long-api-key-value"),
        ];
        for bind in ["127.0.0.1:8080", "0.0.0.0:8080"] {
            for key in keys {
                let cfg = config(bind, key);
                let refused = cfg.clone().resolve_api_key().is_err();
                let rejected = cfg.check_api_key().is_err();
                if key == Some(crate::secrets::REDACTED) {
                    assert!(refused && !rejected, "{bind} / {key:?}: redacted");
                } else {
                    assert_eq!(refused, rejected, "{bind} / {key:?}");
                }
            }
        }
    }

    #[test]
    fn the_redaction_placeholder_is_never_a_key() {
        let mut cfg = config("127.0.0.1:8080", Some(crate::secrets::REDACTED));
        let error = cfg
            .resolve_api_key()
            .expect_err("the placeholder must not authenticate anyone");
        assert!(error.contains(crate::secrets::REDACTED));
    }

    #[test]
    fn the_dashboard_follows_the_api_unless_it_is_configured() {
        let off: WebConfig = toml::from_str("enable_api = false").expect("valid [web] section");
        assert!(!off.webui_enabled());

        let on: WebConfig = toml::from_str("").expect("valid [web] section");
        assert!(on.webui_enabled());

        let explicit: WebConfig =
            toml::from_str("enable_api = false\nenable_webui = true").expect("valid [web] section");
        assert!(explicit.webui_enabled());
    }
}
