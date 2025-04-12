use serde::Deserialize;
use std::time::Duration;

use super::{
    cache::CacheConfig, filter::FilterConfig, logging::LoggingConfig, manager::ManagerConfig,
    server::ServerMotds, telemetry::TelemetryConfig,
};
use crate::core::config::provider::{docker::DockerProviderConfig, file::FileProviderConfig};

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

        self.logging = other.logging;
    }
}
