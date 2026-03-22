use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerWakeConfig {
    #[serde(default)]
    pub timing: TimingConfig,
    #[serde(default)]
    pub messages: ServerWakeMessages,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingConfig {
    #[serde(default = "default_start_timeout")]
    pub start_timeout_seconds: u64,
    #[serde(default = "default_title_refresh")]
    pub title_refresh_interval_seconds: u64,
    #[serde(default = "default_true")]
    pub show_waiting_count: bool,
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            start_timeout_seconds: default_start_timeout(),
            title_refresh_interval_seconds: default_title_refresh(),
            show_waiting_count: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerWakeMessages {
    #[serde(default = "default_starting_title")]
    pub starting_title: String,
    #[serde(default = "default_starting_subtitle")]
    pub starting_subtitle: String,
    #[serde(default = "default_stopping_title")]
    pub stopping_title: String,
    #[serde(default = "default_stopping_subtitle")]
    pub stopping_subtitle: String,
    #[serde(default = "default_ready_title")]
    pub ready_title: String,
    #[serde(default = "default_ready_subtitle")]
    pub ready_subtitle: String,
    #[serde(default = "default_failed_kick")]
    pub failed_kick: String,
    #[serde(default = "default_timeout_kick")]
    pub timeout_kick: String,
    #[serde(default = "default_waiting_action_bar")]
    pub waiting_action_bar: String,
}

impl Default for ServerWakeMessages {
    fn default() -> Self {
        Self {
            starting_title: default_starting_title(),
            starting_subtitle: default_starting_subtitle(),
            stopping_title: default_stopping_title(),
            stopping_subtitle: default_stopping_subtitle(),
            ready_title: default_ready_title(),
            ready_subtitle: default_ready_subtitle(),
            failed_kick: default_failed_kick(),
            timeout_kick: default_timeout_kick(),
            waiting_action_bar: default_waiting_action_bar(),
        }
    }
}

const fn default_start_timeout() -> u64 {
    180
}
const fn default_title_refresh() -> u64 {
    3
}
const fn default_true() -> bool {
    true
}

fn default_starting_title() -> String {
    "&eServer Starting".into()
}
fn default_starting_subtitle() -> String {
    "&7Please wait&f{dots}".into()
}
fn default_stopping_title() -> String {
    "&eServer Restarting".into()
}
fn default_stopping_subtitle() -> String {
    "&7Waiting for shutdown&f{dots}".into()
}
fn default_ready_title() -> String {
    "&aServer Ready!".into()
}
fn default_ready_subtitle() -> String {
    "&7Connecting you now...".into()
}
fn default_failed_kick() -> String {
    "&cThe server failed to start. Please try again later.".into()
}
fn default_timeout_kick() -> String {
    "&cThe server took too long to start. Please try again.".into()
}
fn default_waiting_action_bar() -> String {
    "&7{count} player(s) waiting for &e{server}".into()
}

pub async fn load_or_create_config(path: &Path) -> Result<ServerWakeConfig, String> {
    if path.exists() {
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| format!("failed to read config: {e}"))?;
        toml::from_str(&content).map_err(|e| format!("failed to parse config: {e}"))
    } else {
        let config = ServerWakeConfig::default();
        let content =
            toml::to_string_pretty(&config).map_err(|e| format!("failed to serialize config: {e}"))?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("failed to create config dir: {e}"))?;
        }
        tokio::fs::write(path, &content)
            .await
            .map_err(|e| format!("failed to write config: {e}"))?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn test_default_config_valid() {
        let config = ServerWakeConfig::default();
        assert_eq!(config.timing.start_timeout_seconds, 180);
        assert_eq!(config.timing.title_refresh_interval_seconds, 3);
        assert!(config.timing.show_waiting_count);
        assert!(!config.messages.starting_title.is_empty());
        assert!(!config.messages.failed_kick.is_empty());
    }

    #[test]
    fn test_config_roundtrip() {
        let config = ServerWakeConfig::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: ServerWakeConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(
            deserialized.timing.start_timeout_seconds,
            config.timing.start_timeout_seconds
        );
        assert_eq!(
            deserialized.messages.starting_title,
            config.messages.starting_title
        );
    }

    #[test]
    fn test_placeholder_substitution() {
        use infrarust_api::types::format_placeholders;
        let msg = format_placeholders(
            "{count} player(s) waiting for {server}",
            &[("count", "3"), ("server", "survival")],
        );
        assert_eq!(msg, "3 player(s) waiting for survival");
    }

    #[test]
    fn test_placeholder_missing_key_left_as_is() {
        use infrarust_api::types::format_placeholders;
        let msg = format_placeholders("Hello {name}", &[("other", "value")]);
        assert_eq!(msg, "Hello {name}");
    }

    #[test]
    fn test_placeholder_dots() {
        use infrarust_api::types::format_placeholders;
        let msg = format_placeholders("Starting{dots}", &[("dots", "...")]);
        assert_eq!(msg, "Starting...");
    }
}
