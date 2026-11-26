use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const MAX_RESTART_DELAY: Duration = Duration::from_secs(60 * 60);
pub const MAX_RESTART_MESSAGE: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartLimit {
    Delay { delay: Duration, max: Duration },
    Message { length: usize, max: usize },
}

impl RestartLimit {
    pub fn check(delay: Option<Duration>, message: Option<&str>) -> Result<(), Self> {
        if let Some(delay) = delay
            && delay > MAX_RESTART_DELAY
        {
            return Err(Self::Delay {
                delay,
                max: MAX_RESTART_DELAY,
            });
        }
        let length = message.map_or(0, |message| message.chars().count());
        if length > MAX_RESTART_MESSAGE {
            return Err(Self::Message {
                length,
                max: MAX_RESTART_MESSAGE,
            });
        }
        Ok(())
    }
}

impl fmt::Display for RestartLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Delay { delay, max } => write!(
                f,
                "the delay is {}, the longest delay is {}",
                humantime::format_duration(*delay),
                humantime::format_duration(*max)
            ),
            Self::Message { length, max } => write!(
                f,
                "the message is {length} characters long, the limit is {max}"
            ),
        }
    }
}

impl std::error::Error for RestartLimit {}

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
    fn a_delay_or_a_message_over_the_limit_is_named() {
        let hour = Duration::from_secs(3600);
        assert_eq!(RestartLimit::check(None, None), Ok(()));
        assert_eq!(RestartLimit::check(Some(hour), None), Ok(()));
        let late = RestartLimit::check(Some(hour + Duration::from_secs(1)), None).unwrap_err();
        assert_eq!(
            late.to_string(),
            "the delay is 1h 1s, the longest delay is 1h"
        );
        let fits = "é".repeat(MAX_RESTART_MESSAGE);
        assert_eq!(RestartLimit::check(None, Some(&fits)), Ok(()));
        let long = "é".repeat(MAX_RESTART_MESSAGE + 1);
        assert_eq!(
            RestartLimit::check(Some(hour), Some(&long))
                .unwrap_err()
                .to_string(),
            "the message is 257 characters long, the limit is 256"
        );
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(toml::from_str::<RestartConfig>("countdown = true").is_err());
    }
}
