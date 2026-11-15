//! Web admin API / UI configuration.

use serde::{Deserialize, Serialize};

use crate::defaults;

fn default_bind() -> String {
    "127.0.0.1:8080".to_string()
}

fn default_requests_per_minute() -> u64 {
    60
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebConfig {
    #[serde(default = "defaults::true_val")]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKeyState {
    Usable(String),
    MissingOnLoopback,
    Redacted,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ApiKeyError {
    #[error(
        "API key is too short ({len} chars). Minimum length is {MIN_API_KEY_LENGTH} characters."
    )]
    TooShort { len: usize },

    #[error(
        "the web admin API is bound to a non-loopback address ({bind}) but no `api_key` is set. \
         Set a strong `api_key` (>= {MIN_API_KEY_LENGTH} characters) in the [web] section, \
         or bind to 127.0.0.1. Refusing to start an externally-reachable admin API without authentication."
    )]
    MissingOnPublicBind { bind: String },

    #[error(
        "`api_key` in the [web] section is the redaction placeholder '{}'. \
         It is not a key: put the real one back.",
        crate::secrets::REDACTED
    )]
    Redacted,
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

    pub fn api_key_state(&self) -> Result<ApiKeyState, ApiKeyError> {
        match self.api_key.as_deref() {
            Some(key) if key == crate::secrets::REDACTED => Ok(ApiKeyState::Redacted),
            Some(key) if key != "CHANGE-ME" && !key.is_empty() => {
                if key.len() < MIN_API_KEY_LENGTH {
                    return Err(ApiKeyError::TooShort { len: key.len() });
                }
                Ok(ApiKeyState::Usable(key.to_string()))
            }
            _ if !self.bind_is_loopback() => Err(ApiKeyError::MissingOnPublicBind {
                bind: self.bind.clone(),
            }),
            _ => Ok(ApiKeyState::MissingOnLoopback),
        }
    }

    /// Whether the admin API can start with this key, so a configuration that
    /// cannot boot is refused before it is written.
    ///
    /// A redacted key passes: it stands for a stored key this config does not
    /// carry, and a write puts the real one back.
    ///
    /// # Errors
    ///
    /// Returns the message of the [`ApiKeyError`] that
    /// [`api_key_state`](Self::api_key_state) fails with.
    pub fn check_api_key(&self) -> Result<(), String> {
        self.api_key_state()
            .map(drop)
            .map_err(|error| error.to_string())
    }

    /// The key the admin API authenticates with, minting an ephemeral one for
    /// a loopback bind that configures none.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when the configured key cannot be used.
    #[deprecated(
        note = "match on `api_key_state` and generate the ephemeral key in the caller; \
                this function prints to stdout and mutates the config"
    )]
    pub fn resolve_api_key(&mut self) -> Result<String, String> {
        match self.api_key_state() {
            Ok(ApiKeyState::Usable(key)) => Ok(key),
            Ok(ApiKeyState::Redacted) => Err(ApiKeyError::Redacted.to_string()),
            Err(error) => Err(error.to_string()),
            Ok(ApiKeyState::MissingOnLoopback) => {
                let generated = uuid::Uuid::new_v4().to_string();
                tracing::warn!(
                    "No API key configured for loopback bind ({}) — generated an ephemeral key, \
                     printed once on stdout. Set `api_key` in the [web] section to keep it across restarts.",
                    self.bind
                );
                println!(
                    "[web] ephemeral API key for {}: {generated}\n\
                     It changes on every restart and is not written to the configuration file.",
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
            enable_api: defaults::true_val(),
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn config(bind: &str, api_key: Option<&str>) -> WebConfig {
        WebConfig {
            bind: bind.to_string(),
            api_key: api_key.map(str::to_string),
            ..WebConfig::default()
        }
    }

    #[test]
    fn loopback_without_key_is_missing() {
        for bind in ["127.0.0.1:8080", "localhost:8080", "[::1]:8080"] {
            assert_eq!(
                config(bind, None).api_key_state(),
                Ok(ApiKeyState::MissingOnLoopback),
                "{bind}"
            );
        }
    }

    #[test]
    fn non_loopback_without_key_is_hard_error() {
        for bind in ["0.0.0.0:8080", "192.168.1.10:8080", "[::]:8080"] {
            for key in [None, Some(""), Some("CHANGE-ME")] {
                assert_eq!(
                    config(bind, key).api_key_state(),
                    Err(ApiKeyError::MissingOnPublicBind {
                        bind: bind.to_string()
                    }),
                    "{bind} with {key:?} must refuse to start"
                );
            }
        }
    }

    #[test]
    fn short_key_is_refused_on_any_bind() {
        for bind in ["127.0.0.1:8080", "0.0.0.0:8080"] {
            assert_eq!(
                config(bind, Some("hunter2")).api_key_state(),
                Err(ApiKeyError::TooShort { len: 7 })
            );
        }
    }

    #[test]
    fn non_loopback_with_strong_key_is_accepted() {
        let cfg = config("0.0.0.0:8080", Some("a-sufficiently-long-api-key-value"));
        assert_eq!(
            cfg.api_key_state(),
            Ok(ApiKeyState::Usable(
                "a-sufficiently-long-api-key-value".to_string()
            ))
        );
    }

    #[test]
    fn a_redacted_key_is_reported_and_passes_the_write_check() {
        let cfg = config("0.0.0.0:8080", Some(crate::secrets::REDACTED));
        assert_eq!(cfg.api_key_state(), Ok(ApiKeyState::Redacted));
        assert!(cfg.check_api_key().is_ok());
        assert!(
            ApiKeyError::Redacted
                .to_string()
                .contains(crate::secrets::REDACTED)
        );
    }

    #[test]
    fn check_reports_the_typed_error_message() {
        let cfg = config("0.0.0.0:8080", None);
        assert_eq!(
            cfg.check_api_key().unwrap_err(),
            cfg.api_key_state().unwrap_err().to_string()
        );
    }

    #[test]
    #[allow(deprecated)]
    fn deprecated_resolve_still_mints_and_refuses() {
        let mut cfg = config("127.0.0.1:8080", None);
        let key = cfg.resolve_api_key().expect("loopback may auto-generate");
        assert_eq!(cfg.api_key.as_deref(), Some(key.as_str()));

        let error = config("127.0.0.1:8080", Some(crate::secrets::REDACTED))
            .resolve_api_key()
            .expect_err("the placeholder must not authenticate anyone");
        assert!(error.contains(crate::secrets::REDACTED));

        assert!(config("0.0.0.0:8080", None).resolve_api_key().is_err());
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
