pub mod provider;
pub mod service;
use std::time::Duration;

use provider::file::FileProviderConfig;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    proxy_modes::ProxyModeEnum, security::filter::RateLimiterConfig, server::motd::MotdConfig,
};

#[derive(Debug, Clone, Deserialize)]
pub struct AccessListConfig<T> {
    pub enabled: bool,
    pub whitelist: Vec<T>,
    pub blacklist: Vec<T>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilterConfig {
    pub rate_limiter: Option<RateLimiterConfig>,
    pub ip_filter: Option<AccessListConfig<String>>,
    pub id_filter: Option<AccessListConfig<Uuid>>,
    pub name_filter: Option<AccessListConfig<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatusCacheOptions {
    pub enabled: bool,
    pub ttl: Duration,
    pub max_size: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    pub status: Option<StatusCacheOptions>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub export_interval_seconds: u64,
    #[serde(default)]
    pub export_url: Option<String>,
    #[serde(default)]
    pub enable_metrics: bool,
    #[serde(default)]
    pub enable_tracing: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        TelemetryConfig {
            enabled: false,
            export_interval_seconds: 30,
            export_url: None,
            enable_metrics: false,
            enable_tracing: false,
        }
    }
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
            proxy_protocol_version: Some(2),
        }
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

#[derive(Debug, Deserialize, Clone, Default)]
pub struct InfrarustConfig {
    pub bind: Option<String>,
    pub domains: Option<Vec<String>>,
    pub addresses: Option<Vec<String>>,
    pub keepalive_timeout: Option<Duration>,
    pub file_provider: Option<FileProviderConfig>,

    #[serde(default)]
    pub telemetry: TelemetryConfig,

    #[serde(default)]
    pub motds: ServerMotds,
}

impl ServerConfig {
    pub fn is_empty(&self) -> bool {
        self.domains.is_empty() && self.addresses.is_empty()
    }
}

impl InfrarustConfig {
    pub fn is_empty(&self) -> bool {
        self.bind.is_none() && self.domains.is_none() && self.addresses.is_none()
    }

    pub fn merge(&mut self, other: &InfrarustConfig) {
        if let Some(bind) = &other.bind {
            self.bind = Some(bind.clone());
        }

        if let Some(domains) = &other.domains {
            self.domains = Some(domains.clone());
        }

        if let Some(addresses) = &other.addresses {
            self.addresses = Some(addresses.clone());
        }

        if let Some(keepalive_timeout) = &other.keepalive_timeout {
            self.keepalive_timeout = Some(*keepalive_timeout);
        }

        if let Some(file_provider) = &other.file_provider {
            self.file_provider = Some(file_provider.clone());
        }

        if other.motds.unknown.is_some() {
            self.motds.unknown = other.motds.unknown.clone();
        }

        if other.motds.unreachable.is_some() {
            self.motds.unreachable = other.motds.unreachable.clone();
        }

        if other.telemetry.enabled {
            self.telemetry = other.telemetry.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    #[test]
    fn test_file_provider() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");
        let proxies_path = temp_dir.path().join("proxies");

        fs::create_dir(&proxies_path).unwrap();

        fs::write(&config_path, "bind: ':25565'\n").unwrap();
        fs::write(
            proxies_path.join("server1.yml"),
            "domains: ['example.com']\naddresses: ['127.0.0.1:25566']\n",
        )
        .unwrap();

        // let provider = FileProvider::new(
        //     config_path.to_str().unwrap().to_string(),
        //     proxies_path.to_str().unwrap().to_string(),
        //     FileType::Yaml,
        // );

        // let config = provider.load_config().unwrap();
        // assert!(!config.server_configs.is_empty());
    }
}
