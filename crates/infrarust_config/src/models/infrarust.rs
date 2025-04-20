use serde::Deserialize;
use std::time::Duration;

use super::{
    cache::CacheConfig, filter::FilterConfig, logging::LoggingConfig, manager::ManagerConfig,
    server::ServerMotds, telemetry::TelemetryConfig,
};

// File provider and docker provider will be defined in provider module
#[derive(Debug, Deserialize, Clone)]
pub struct FileProviderConfig {
    #[serde(default)]
    pub proxies_path: Vec<String>,
    #[serde(default)]
    pub file_type: FileType,
    #[serde(default)]
    pub watch: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
pub enum FileType {
    #[serde(rename = "yaml")]
    #[default]
    Yaml,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DockerProviderConfig {
    #[serde(default)]
    pub docker_host: String,
    #[serde(default)]
    pub label_prefix: String,
    #[serde(default)]
    pub polling_interval: u64,
    #[serde(default)]
    pub watch: bool,
    #[serde(default)]
    pub default_domains: Vec<String>,
}

impl Default for DockerProviderConfig {
    fn default() -> Self {
        Self {
            docker_host: "unix:///var/run/docker.sock".to_string(),
            label_prefix: "infrarust".to_string(),
            polling_interval: 10,
            watch: true,
            default_domains: Vec::new(),
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
    pub docker_provider: Option<DockerProviderConfig>,

    pub managers_config: Option<ManagerConfig>,

    #[serde(default)]
    pub cache: CacheConfig,

    #[serde(default)]
    pub filters: Option<FilterConfig>,

    #[serde(default)]
    pub telemetry: TelemetryConfig,

    #[serde(default)]
    pub logging: LoggingConfig,

    #[serde(default)]
    pub motds: ServerMotds,

    #[serde(default)]
    pub proxy_protocol: Option<ProxyProtocolConfig>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct ProxyProtocolConfig {
    pub enabled: bool,
    /// Version to use for outgoing proxy protocol (1 or 2)
    pub version: Option<u8>,
    /// Enable receiving proxy protocol headers from clients
    pub receive_enabled: bool,
    /// Timeout in seconds for receiving proxy protocol headers
    pub receive_timeout_secs: Option<u64>,
    /// Allowed proxy protocol versions for incoming connections (1, 2, or both)
    pub receive_allowed_versions: Option<Vec<u8>>,
}

impl InfrarustConfig {
    pub fn is_empty(&self) -> bool {
        self.bind.is_none() && self.domains.is_none() && self.addresses.is_none()
    }

    pub fn merge(&mut self, other: InfrarustConfig) {
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

        if let Some(docker_provider) = &other.docker_provider {
            self.docker_provider = Some(docker_provider.clone());
        }

        if let Some(manager_config) = &other.managers_config {
            self.managers_config = Some(manager_config.clone());
        }

        if other.motds.unknown.is_some() {
            self.motds.unknown = other.motds.unknown;
        }

        if other.motds.unreachable.is_some() {
            self.motds.unreachable = other.motds.unreachable;
        }

        if other.telemetry.enabled {
            self.telemetry = other.telemetry;
        }

        if other.filters.is_some() {
            self.filters = other.filters;
        }

        if other.proxy_protocol.is_some() {
            self.proxy_protocol = other.proxy_protocol;
        }

        self.logging = other.logging;
    }
}
