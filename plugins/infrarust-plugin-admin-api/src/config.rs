use std::path::Path;

use axum::http::HeaderValue;
use infrarust_api::error::PluginError;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    #[serde(default = "ApiConfig::default_bind")]
    pub bind: String,

    pub api_key: String,

    #[serde(default)]
    pub cors_origins: Vec<String>,

    #[serde(default)]
    pub rate_limit: RateLimitConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default = "RateLimitConfig::default_rpm")]
    pub requests_per_minute: u64,
}

impl RateLimitConfig {
    fn default_rpm() -> u64 {
        60
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: Self::default_rpm(),
        }
    }
}

impl ApiConfig {
    fn default_bind() -> String {
        "127.0.0.1:8080".to_string()
    }

    /// Constant-time API key verification.
    /// Both values are right-padded to equal length to prevent length-leaking.
    pub fn verify_api_key(&self, token: &str) -> bool {
        use subtle::ConstantTimeEq;

        let key = self.api_key.as_bytes();
        let tok = token.as_bytes();
        let len = key.len().max(tok.len());

        let mut key_padded = vec![0u8; len];
        let mut tok_padded = vec![0u8; len];
        key_padded[..key.len()].copy_from_slice(key);
        tok_padded[..tok.len()].copy_from_slice(tok);

        let len_eq = key.len() == tok.len();
        let content_eq: bool = key_padded.ct_eq(&tok_padded).into();

        len_eq & content_eq
    }
}

const DEFAULT_CONFIG: &str = r#"# Admin API configuration
# Documentation: https://github.com/Shadowner/Infrarust/wiki/admin-api

bind = "127.0.0.1:8080"

# IMPORTANT: Change this API key before exposing the API
api_key = "CHANGE-ME"

# CORS origins for the web dashboard (empty = no CORS)
# cors_origins = ["http://localhost:3000"]

# Rate limiting (requests per minute for authenticated endpoints)
# [rate_limit]
# requests_per_minute = 60
"#;

fn generate_api_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub async fn load_config(data_dir: &Path) -> Result<ApiConfig, PluginError> {
    let config_path = data_dir.join("config.toml");

    if !config_path.exists() {
        tokio::fs::create_dir_all(data_dir)
            .await
            .map_err(|e| PluginError::InitFailed(format!("Failed to create data dir: {e}")))?;

        let generated_key = generate_api_key();
        let config_content = DEFAULT_CONFIG.replace("CHANGE-ME", &generated_key);

        tokio::fs::write(&config_path, &config_content)
            .await
            .map_err(|e| PluginError::InitFailed(format!("Failed to write default config: {e}")))?;

        tracing::info!("Generated admin API key: {generated_key}");
        tracing::info!("Config written to {}", config_path.display());
    }

    let content = tokio::fs::read_to_string(&config_path)
        .await
        .map_err(|e| PluginError::InitFailed(format!("Failed to read config: {e}")))?;

    let mut config: ApiConfig = toml::from_str(&content)
        .map_err(|e| PluginError::InitFailed(format!("Invalid config: {e}")))?;

    if config.api_key == "CHANGE-ME" {
        let generated_key = generate_api_key();
        let updated_content = content.replace("CHANGE-ME", &generated_key);

        tokio::fs::write(&config_path, &updated_content)
            .await
            .map_err(|e| PluginError::InitFailed(format!("Failed to update config: {e}")))?;

        config.api_key = generated_key.clone();
        tracing::info!("Generated admin API key: {generated_key}");
    }

    const MIN_KEY_LENGTH: usize = 16;
    if config.api_key.len() < MIN_KEY_LENGTH {
        return Err(PluginError::InitFailed(format!(
            "API key is too short ({} chars). Minimum length is {MIN_KEY_LENGTH} characters.",
            config.api_key.len()
        )));
    }

    for origin in &config.cors_origins {
        if origin.parse::<HeaderValue>().is_err() {
            tracing::warn!(origin = %origin, "Ignoring invalid CORS origin");
        }
    }

    Ok(config)
}
