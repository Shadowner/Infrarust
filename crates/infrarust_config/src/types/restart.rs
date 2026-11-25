use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const MAX_RESTART_DELAY: Duration = Duration::from_secs(60 * 60);
pub const MAX_RESTART_MESSAGE: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RestartConfig {
    #[serde(default = "crate::defaults::true_val")]
    pub transfer: bool,

    #[serde(default)]
    pub reauthenticate: bool,

    #[serde(default)]
    pub message: String,

    #[serde(default, with = "humantime_serde")]
    pub delay: Duration,
}

impl Default for RestartConfig {
    fn default() -> Self {
        Self {
            transfer: true,
            reauthenticate: false,
            message: String::new(),
            delay: Duration::ZERO,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn an_empty_section_is_fully_defaulted() {
        let config: RestartConfig = toml::from_str("").unwrap();
        assert_eq!(config, RestartConfig::default());
        assert!(config.transfer);
        assert!(!config.reauthenticate);
        assert!(config.message.is_empty());
        assert_eq!(config.delay, Duration::ZERO);
    }

    #[test]
    fn every_field_parses() {
        let config: RestartConfig = toml::from_str(
            "transfer = false\nreauthenticate = true\nmessage = \"&eRestart in {delay}\"\ndelay = \"30s\"\n",
        )
        .unwrap();
        assert!(!config.transfer);
        assert!(config.reauthenticate);
        assert_eq!(config.message, "&eRestart in {delay}");
        assert_eq!(config.delay, Duration::from_secs(30));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(toml::from_str::<RestartConfig>("countdown = true").is_err());
    }
}
