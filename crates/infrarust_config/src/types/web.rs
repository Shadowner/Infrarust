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
            _ => {
                let generated = uuid::Uuid::new_v4().to_string();
                tracing::warn!("No API key configured — generated key: {generated}");
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
