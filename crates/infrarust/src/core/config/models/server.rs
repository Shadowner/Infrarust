use serde::Deserialize;

use crate::{proxy_modes::ProxyModeEnum, server::motd::MotdConfig};
use super::{cache::CacheConfig, filter::FilterConfig};

#[derive(Debug, Clone, Deserialize)]
pub struct ServerManagerConfig {
    pub provider_name: String,
    pub empty_timeout: Option<u64>,
    pub launch_command: Option<String>
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub domains: Vec<String>,
    pub addresses: Vec<String>,
    #[serde(rename = "sendProxyProtocol")]
    pub send_proxy_protocol: Option<bool>,
    #[serde(rename = "proxyMode")]
    pub proxy_mode: Option<ProxyModeEnum>,
    pub filters: Option<FilterConfig>,
    pub caches: Option<CacheConfig>,

    pub motd: Option<MotdConfig>,
    pub server_manager: Option<ServerManagerConfig>,

    #[serde(rename = "configId", default)]
    pub config_id: String,
    pub proxy_protocol_version: Option<u8>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            domains: Vec::new(),
            addresses: Vec::new(),
            send_proxy_protocol: Some(false),
            proxy_mode: Some(ProxyModeEnum::default()),
            config_id: String::new(),
            filters: None,
            caches: None,
            motd: None,
            server_manager: None,
            proxy_protocol_version: Some(2),
        }
    }
}

impl ServerConfig {
    pub fn is_empty(&self) -> bool {
        self.domains.is_empty() && self.addresses.is_empty()
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerMotds {
    pub unknown: Option<MotdConfig>,
    pub unreachable: Option<MotdConfig>,
}

impl Default for ServerMotds {
    fn default() -> Self {
        ServerMotds {
            unknown: Some(MotdConfig::default()),
            unreachable: Some(MotdConfig::default_unreachable()),
        }
    }
}