//! Web admin API / UI configuration.

use serde::Deserialize;

fn default_true() -> bool {
    true
}

fn default_listen_port() -> u16 {
    8080
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebConfig {
    #[serde(default = "default_true")]
    pub enable_api: bool,

    #[serde(default = "default_true")]
    pub enable_webui: bool,

    #[serde(default = "default_listen_port")]
    pub listen_port: u16,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            enable_api: true,
            enable_webui: true,
            listen_port: 8080,
        }
    }
}
