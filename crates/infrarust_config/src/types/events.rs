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
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            handler_timeout: defaults::event_handler_timeout(),
            slow_handler_threshold: defaults::event_slow_handler_threshold(),
            packet_handler_timeout: defaults::event_packet_handler_timeout(),
        }
    }
}
