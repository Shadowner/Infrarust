use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::defaults;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventsConfig {
    #[serde(default = "defaults::event_handler_timeout")]
    #[serde(with = "humantime_serde")]
    pub handler_timeout: Duration,

    #[serde(default = "defaults::event_slow_handler_threshold")]
    #[serde(with = "humantime_serde")]
    pub slow_handler_threshold: Duration,

    #[serde(default = "defaults::event_packet_handler_timeout")]
    #[serde(with = "humantime_serde")]
    pub packet_handler_timeout: Duration,

    #[serde(default = "defaults::event_disconnect_deadline")]
    #[serde(with = "humantime_serde")]
    pub disconnect_deadline: Duration,

    #[serde(default = "defaults::event_transport_filter_timeout")]
    #[serde(with = "humantime_serde")]
    pub transport_filter_timeout: Duration,
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            handler_timeout: defaults::event_handler_timeout(),
            slow_handler_threshold: defaults::event_slow_handler_threshold(),
            packet_handler_timeout: defaults::event_packet_handler_timeout(),
            disconnect_deadline: defaults::event_disconnect_deadline(),
            transport_filter_timeout: defaults::event_transport_filter_timeout(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnect_deadline_defaults_to_fifteen_seconds() {
        let config: EventsConfig = toml::from_str("").expect("empty events config");
        assert_eq!(config.disconnect_deadline, Duration::from_secs(15));
    }

    #[test]
    fn parses_a_custom_disconnect_deadline() {
        let config: EventsConfig =
            toml::from_str(r#"disconnect_deadline = "300ms""#).expect("valid events config");
        assert_eq!(config.disconnect_deadline, Duration::from_millis(300));
    }

    #[test]
    fn transport_filter_timeout_defaults_to_five_seconds() {
        let config: EventsConfig = toml::from_str("").expect("empty events config");
        assert_eq!(config.transport_filter_timeout, Duration::from_secs(5));
    }

    #[test]
    fn parses_a_custom_transport_filter_timeout() {
        let config: EventsConfig =
            toml::from_str(r#"transport_filter_timeout = "250ms""#).expect("valid events config");
        assert_eq!(config.transport_filter_timeout, Duration::from_millis(250));
    }
}
