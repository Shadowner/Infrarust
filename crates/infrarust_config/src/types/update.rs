use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const DEFAULT_UPDATE_URL: &str = "https://api.github.com/repos/Shadowner/Infrarust/releases";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    #[default]
    Auto,
    Stable,
    Prerelease,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfig {
    #[serde(default = "crate::defaults::true_val")]
    pub check: bool,

    #[serde(default)]
    pub channel: UpdateChannel,

    #[serde(default = "defaults::interval", with = "humantime_serde")]
    pub interval: Duration,

    #[serde(default = "defaults::url")]
    pub url: String,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            check: true,
            channel: UpdateChannel::default(),
            interval: defaults::interval(),
            url: defaults::url(),
        }
    }
}

mod defaults {
    use std::time::Duration;

    pub const fn interval() -> Duration {
        Duration::from_secs(24 * 60 * 60)
    }

    pub fn url() -> String {
        super::DEFAULT_UPDATE_URL.to_string()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn an_empty_section_is_fully_defaulted() {
        let config: UpdateConfig = toml::from_str("").unwrap();
        assert_eq!(config, UpdateConfig::default());
        assert!(config.check);
        assert_eq!(config.channel, UpdateChannel::Auto);
        assert_eq!(config.interval, Duration::from_secs(86_400));
    }

    #[test]
    fn every_field_parses() {
        let config: UpdateConfig = toml::from_str(
            "check = false\nchannel = \"prerelease\"\ninterval = \"6h\"\nurl = \"https://mirror.example/releases\"\n",
        )
        .unwrap();
        assert!(!config.check);
        assert_eq!(config.channel, UpdateChannel::Prerelease);
        assert_eq!(config.interval, Duration::from_secs(6 * 3600));
        assert_eq!(config.url, "https://mirror.example/releases");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(toml::from_str::<UpdateConfig>("auto_install = true").is_err());
    }
}
