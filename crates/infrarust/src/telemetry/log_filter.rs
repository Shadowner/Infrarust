use std::collections::HashMap;
use tracing::{Level, Metadata};
use tracing_subscriber::filter::FilterFn;
use infrarust_config::{LoggingConfig, LogType};

use super::log_type_layer::LogTypeStorage;

pub struct InfrarustLogFilter {
    type_levels: HashMap<String, Level>,
    excluded_types: Vec<String>,
    min_level: Level,
    default_level: Level,
    log_type_storage: Option<LogTypeStorage>,
}

impl InfrarustLogFilter {
    pub fn from_config(config: &LoggingConfig) -> Self {
        let mut type_levels = HashMap::new();
        
        for (log_type, level_str) in &config.log_types {
            if let Ok(level) = parse_level(level_str) {
                type_levels.insert(log_type.clone(), level);
            }
        }

        let min_level = config.min_level.as_ref()
            .and_then(|l| parse_level(l).ok())
            .unwrap_or(if cfg!(debug_assertions) || config.debug { 
                Level::DEBUG 
            } else { 
                Level::INFO 
            });

        let default_level = min_level;

        Self {
            type_levels,
            excluded_types: config.exclude_types.clone(),
            min_level,
            default_level,
            log_type_storage: None,
        }
    }

    pub fn with_log_type_storage(mut self, storage: LogTypeStorage) -> Self {
        self.log_type_storage = Some(storage);
        self
    }

    pub fn should_log(&self, metadata: &Metadata<'_>) -> bool {
        let log_type = self.extract_log_type_from_storage()
            .or_else(|| extract_log_type_from_target(metadata.target()));

        if let Some(log_type) = log_type {
            if self.excluded_types.contains(&log_type) {
                return false;
            }

            if let Some(&type_level) = self.type_levels.get(&log_type) {
                let required_level = std::cmp::max(type_level, self.min_level);
                return metadata.level() <= &required_level;
            }
        }

        metadata.level() <= &std::cmp::max(self.default_level, self.min_level)
    }

    fn extract_log_type_from_storage(&self) -> Option<String> {
        if let Some(ref storage) = self.log_type_storage {
            storage.get_current_log_type()
        } else {
            None
        }
    }

    pub fn create_filter_fn(self) -> FilterFn<impl Fn(&Metadata<'_>) -> bool> {
        tracing_subscriber::filter::filter_fn(move |metadata| {
            self.should_log(metadata)
        })
    }
}

fn extract_log_type_from_target(target: &str) -> Option<String> {
    match target {
        // Core system components
        t if t.contains("network::connection") || t.contains("connection") => 
            Some(LogType::TcpConnection.as_str().to_string()),
        t if t.contains("core::actors::supervisor") || t.contains("supervisor") => 
            Some(LogType::Supervisor.as_str().to_string()),
        t if t.contains("server::manager") || t.contains("server_manager") => 
            Some(LogType::ServerManager.as_str().to_string()),
        
        // Protocol and packet handling - check proxy_protocol first (more specific)
        t if t.contains("proxy_protocol") => 
            Some(LogType::ProxyProtocol.as_str().to_string()),
        t if t.contains("protocol") || t.contains("packet") => 
            Some(LogType::PacketProcessing.as_str().to_string()),
        
        // Security and filtering
        t if t.contains("ban") => 
            Some(LogType::BanSystem.as_str().to_string()),
        t if t.contains("auth") || t.contains("encryption") => 
            Some(LogType::Authentication.as_str().to_string()),
        t if t.contains("filter") || t.contains("rate_limit") => 
            Some(LogType::Filter.as_str().to_string()),
        
        // Configuration and caching
        t if t.contains("config") || t.contains("provider") => 
            Some(LogType::ConfigProvider.as_str().to_string()),
        t if t.contains("cache") => 
            Some(LogType::Cache.as_str().to_string()),
        
        // Services and features
        t if t.contains("telemetry") || t.contains("metrics") => 
            Some(LogType::Telemetry.as_str().to_string()),
        t if t.contains("motd") => 
            Some(LogType::Motd.as_str().to_string()),
        
        // Gateway and server components
        t if t.contains("gateway") => 
            Some(LogType::ServerManager.as_str().to_string()),
        t if t.contains("backend") => 
            Some(LogType::ServerManager.as_str().to_string()),
        
        // Default case
        _ => None,
    }
}

fn parse_level(level_str: &str) -> Result<Level, &'static str> {
    match level_str.to_lowercase().as_str() {
        "trace" => Ok(Level::TRACE),
        "debug" => Ok(Level::DEBUG),
        "info" => Ok(Level::INFO),
        "warn" | "warning" => Ok(Level::WARN),
        "error" => Ok(Level::ERROR),
        _ => Err("Invalid log level"),
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use tracing::Level;

    #[test]
    fn test_parse_level() {
        assert_eq!(parse_level("debug").unwrap(), Level::DEBUG);
        assert_eq!(parse_level("INFO").unwrap(), Level::INFO);
        assert_eq!(parse_level("warn").unwrap(), Level::WARN);
        assert!(parse_level("invalid").is_err());
    }

    #[test]
    fn test_extract_log_type_from_target() {
        assert_eq!(
            extract_log_type_from_target("infrarust::network::connection"),
            Some("tcp_connection".to_string())
        );
        assert_eq!(
            extract_log_type_from_target("infrarust::core::actors::supervisor"),
            Some("supervisor".to_string())
        );
        assert_eq!(
            extract_log_type_from_target("infrarust::server::manager"),
            Some("server_manager".to_string())
        );
        assert_eq!(
            extract_log_type_from_target("infrarust::network::protocol"),
            Some("packet_processing".to_string())
        );
        assert_eq!(
            extract_log_type_from_target("infrarust::security::ban_system_adapter"),
            Some("ban_system".to_string())
        );
        assert_eq!(
            extract_log_type_from_target("infrarust::security::encryption"),
            Some("authentication".to_string())
        );
        assert_eq!(
            extract_log_type_from_target("infrarust::core::config::provider"),
            Some("config_provider".to_string())
        );
        assert_eq!(
            extract_log_type_from_target("infrarust::server::cache"),
            Some("cache".to_string())
        );
        assert_eq!(
            extract_log_type_from_target("infrarust::security::filter"),
            Some("filter".to_string())
        );
        assert_eq!(
            extract_log_type_from_target("infrarust::network::proxy_protocol"),
            Some("proxy_protocol".to_string())
        );
        assert_eq!(
            extract_log_type_from_target("some::unknown::module"),
            None
        );
    }
    
    #[test]
    fn test_log_filter_with_extracted_types() {
        let mut config = LoggingConfig::default();
        config.log_types.insert("tcp_connection".to_string(), "error".to_string());
        config.log_types.insert("supervisor".to_string(), "debug".to_string());
        config.exclude_types.push("cache".to_string());
        
        let filter = InfrarustLogFilter::from_config(&config);
        
        // Test that the filter was configured correctly
        assert_eq!(filter.type_levels.get("tcp_connection"), Some(&Level::ERROR));
        assert_eq!(filter.type_levels.get("supervisor"), Some(&Level::DEBUG));
        assert!(filter.excluded_types.contains(&"cache".to_string()));
    }

}
