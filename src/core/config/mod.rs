pub mod provider;
pub mod service;
use std::collections::HashMap;
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
pub struct CacheConfig {
    pub status_ttl_seconds: Option<u64>,
    pub max_status_entries: Option<usize>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            status_ttl_seconds: Some(30),
            max_status_entries: Some(1000),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuditLogRotation {
    pub max_size: usize,  // Maximum file size in bytes before rotation
    pub max_files: usize, // Maximum number of rotated files to keep
    pub compress: bool,   // Whether to compress rotated files
}

impl Default for AuditLogRotation {
    fn default() -> Self {
        Self {
            max_size: 10 * 1024 * 1024, // 10MB
            max_files: 5,
            compress: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BanConfig {
    pub enabled: bool,
    pub storage_type: String,
    pub file_path: Option<String>,
    pub redis_url: Option<String>,
    pub database_url: Option<String>,
    pub enable_audit_log: bool,
    pub audit_log_path: Option<String>,
    pub audit_log_rotation: Option<AuditLogRotation>,
    pub auto_cleanup_interval: u64,
    pub cache_size: usize,
}

impl Default for BanConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            storage_type: "file".to_string(),
            file_path: Some("bans.json".to_string()),
            redis_url: None,
            database_url: None,
            enable_audit_log: true,
            audit_log_path: Some("bans_audit.log".to_string()),
            audit_log_rotation: Some(AuditLogRotation::default()),
            auto_cleanup_interval: 3600, // 1 hour
            cache_size: 10_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilterConfig {
    pub rate_limiter: Option<RateLimiterConfig>,
    pub ip_filter: Option<AccessListConfig<String>>,
    pub id_filter: Option<AccessListConfig<Uuid>>,
    pub name_filter: Option<AccessListConfig<String>>,
    #[serde(default)]
    pub ban: BanConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatusCacheOptions {
    pub enabled: bool,
    pub ttl: Duration,
    pub max_size: usize,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LoggingConfig {
    #[serde(default)]
    pub use_color: bool,

    #[serde(default)]
    pub use_icons: bool,

    #[serde(default)]
    pub show_timestamp: bool,

    #[serde(default)]
    pub time_format: String,

    #[serde(default)]
    pub show_target: bool,

    #[serde(default)]
    pub show_fields: bool,

    #[serde(default)]
    pub template: String,

    #[serde(default)]
    pub field_prefixes: HashMap<String, String>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            use_color: true,
            use_icons: true,
            show_timestamp: true,
            time_format: "%Y-%m-%d %H:%M:%S%.3f".to_string(),
            show_target: false,
            show_fields: false,
            template: "{timestamp} {level}: {message}".to_string(),
            field_prefixes: HashMap::new(),
        }
    }
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

impl ServerConfig {
    pub fn is_empty(&self) -> bool {
        self.domains.is_empty() && self.addresses.is_empty()
    }
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
