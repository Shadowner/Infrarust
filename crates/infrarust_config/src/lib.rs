pub mod models;
pub mod provider;

pub use models::access_list::AccessListConfig;
pub use models::ban::{AuditLogRotation, BanConfig};
pub use models::cache::{CacheConfig, StatusCacheOptions};
pub use models::filter::FilterConfig;
pub use models::infrarust::InfrarustConfig;
pub use models::logging::{LoggingConfig, LogType};
pub use models::server::{ServerConfig, ServerManagerConfig, ServerMotds};
pub use models::telemetry::TelemetryConfig;

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
