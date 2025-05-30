use infrarust_config::{LogType, LoggingConfig};
use regex::Regex;
use std::collections::HashMap;
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::{
    filter::FilterFn,
    layer::{Context, Layer},
    registry::LookupSpan,
};

use super::log_type_layer::LogTypeStorage;

pub struct InfrarustLogFilter {
    type_levels: HashMap<String, Level>,
    excluded_types: Vec<String>,
    min_level: Level,
    default_level: Level,
    log_type_storage: Option<LogTypeStorage>,
    regex_filter: Option<Regex>,
}

impl InfrarustLogFilter {
    pub fn from_config(config: &LoggingConfig) -> Self {
        let mut type_levels = HashMap::new();

        for (log_type, level_str) in &config.log_types {
            if let Ok(level) = parse_level(level_str) {
                type_levels.insert(log_type.clone(), level);
            }
        }

        let min_level = config
            .min_level
            .as_ref()
            .and_then(|l| parse_level(l).ok())
            .unwrap_or(if cfg!(debug_assertions) || config.debug {
                Level::DEBUG
            } else {
                Level::INFO
            });

        let default_level = min_level;

        let regex_filter = config
            .regex_filter
            .as_ref()
            .and_then(|pattern| Regex::new(pattern).ok());

        Self {
            type_levels,
            excluded_types: config.exclude_types.clone(),
            min_level,
            default_level,
            log_type_storage: None,
            regex_filter,
        }
    }

    pub fn with_log_type_storage(mut self, storage: LogTypeStorage) -> Self {
        self.log_type_storage = Some(storage);
        self
    }

    pub fn should_log(&self, metadata: &Metadata<'_>) -> bool {
        let log_type = self
            .extract_log_type_from_storage()
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

    pub fn should_log_with_message(&self, metadata: &Metadata<'_>, message: &str) -> bool {
        if !self.should_log(metadata) {
            return false;
        }

        if let Some(ref regex) = self.regex_filter {
            return !regex.is_match(message);
        }

        true
    }

    fn extract_log_type_from_storage(&self) -> Option<String> {
        if let Some(ref storage) = self.log_type_storage {
            storage.get_current_log_type()
        } else {
            None
        }
    }

    pub fn create_filter_fn(self) -> FilterFn<impl Fn(&Metadata<'_>) -> bool> {
        tracing_subscriber::filter::filter_fn(move |metadata| self.should_log(metadata))
    }

    pub fn create_regex_layer(self) -> Option<InfrarustRegexLayer> {
        if self.regex_filter.is_some() {
            Some(InfrarustRegexLayer::new(self))
        } else {
            None
        }
    }

    pub fn get_regex_filter(&self) -> Option<&Regex> {
        self.regex_filter.as_ref()
    }

    pub fn message_matches_regex(&self, message: &str) -> bool {
        if let Some(ref regex) = self.regex_filter {
            regex.is_match(message)
        } else {
            true
        }
    }
}

pub struct InfrarustRegexLayer {
    filter: InfrarustLogFilter,
}

impl InfrarustRegexLayer {
    pub fn new(filter: InfrarustLogFilter) -> Self {
        Self { filter }
    }

    fn extract_event_message(&self, event: &Event<'_>) -> String {
        use std::fmt::Write;
        use tracing::field::{Field, Visit};

        struct MessageCollector {
            message: String,
            fields: Vec<(String, String)>,
        }

        impl Visit for MessageCollector {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                let value_str = format!("{:?}", value);
                if field.name() == "message" {
                    self.message = value_str.trim_matches('"').to_string();
                } else {
                    self.fields.push((field.name().to_string(), value_str));
                }
            }

            fn record_str(&mut self, field: &Field, value: &str) {
                if field.name() == "message" {
                    self.message = value.to_string();
                } else {
                    self.fields
                        .push((field.name().to_string(), value.to_string()));
                }
            }

            fn record_i64(&mut self, field: &Field, value: i64) {
                let value_str = value.to_string();
                if field.name() == "message" {
                    self.message = value_str;
                } else {
                    self.fields.push((field.name().to_string(), value_str));
                }
            }

            fn record_u64(&mut self, field: &Field, value: u64) {
                let value_str = value.to_string();
                if field.name() == "message" {
                    self.message = value_str;
                } else {
                    self.fields.push((field.name().to_string(), value_str));
                }
            }

            fn record_bool(&mut self, field: &Field, value: bool) {
                let value_str = value.to_string();
                if field.name() == "message" {
                    self.message = value_str;
                } else {
                    self.fields.push((field.name().to_string(), value_str));
                }
            }

            fn record_f64(&mut self, field: &Field, value: f64) {
                let value_str = value.to_string();
                if field.name() == "message" {
                    self.message = value_str;
                } else {
                    self.fields.push((field.name().to_string(), value_str));
                }
            }
        }

        let mut collector = MessageCollector {
            message: String::new(),
            fields: Vec::new(),
        };

        event.record(&mut collector);

        // If we have a message field, use it, otherwise construct from all fields
        if !collector.message.is_empty() {
            collector.message
        } else {
            let mut full_message = String::new();
            for (key, value) in collector.fields {
                if !full_message.is_empty() {
                    full_message.push(' ');
                }
                let _ = write!(full_message, "{}={}", key, value);
            }
            full_message
        }
    }
}

impl<S> Layer<S> for InfrarustRegexLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn enabled(&self, metadata: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        self.filter.should_log(metadata)
    }

    fn event_enabled(&self, event: &Event<'_>, ctx: Context<'_, S>) -> bool {
        if !self.enabled(event.metadata(), ctx) {
            return false;
        }

        if self.filter.regex_filter.is_some() {
            let message = self.extract_event_message(event);
            return self
                .filter
                .should_log_with_message(event.metadata(), &message);
        }

        true
    }
}

fn extract_log_type_from_target(target: &str) -> Option<String> {
    match target {
        // Core system components
        t if t.contains("network::connection") || t.contains("connection") => {
            Some(LogType::TcpConnection.as_str().to_string())
        }
        t if t.contains("core::actors::supervisor") || t.contains("supervisor") => {
            Some(LogType::Supervisor.as_str().to_string())
        }
        t if t.contains("server::manager") || t.contains("server_manager") => {
            Some(LogType::ServerManager.as_str().to_string())
        }

        // Protocol and packet handling - check proxy_protocol first (more specific)
        t if t.contains("proxy_protocol") => Some(LogType::ProxyProtocol.as_str().to_string()),
        t if t.contains("protocol") || t.contains("packet") => {
            Some(LogType::PacketProcessing.as_str().to_string())
        }

        // Security and filtering
        t if t.contains("ban") => Some(LogType::BanSystem.as_str().to_string()),
        t if t.contains("auth") || t.contains("encryption") => {
            Some(LogType::Authentication.as_str().to_string())
        }
        t if t.contains("filter") || t.contains("rate_limit") => {
            Some(LogType::Filter.as_str().to_string())
        }

        // Configuration and caching
        t if t.contains("config") || t.contains("provider") => {
            Some(LogType::ConfigProvider.as_str().to_string())
        }
        t if t.contains("cache") => Some(LogType::Cache.as_str().to_string()),

        // Services and features
        t if t.contains("telemetry") || t.contains("metrics") => {
            Some(LogType::Telemetry.as_str().to_string())
        }
        t if t.contains("motd") => Some(LogType::Motd.as_str().to_string()),

        // Gateway and server components
        t if t.contains("gateway") => Some(LogType::ServerManager.as_str().to_string()),
        t if t.contains("backend") => Some(LogType::ServerManager.as_str().to_string()),

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
        assert_eq!(extract_log_type_from_target("some::unknown::module"), None);
    }

    #[test]
    fn test_log_filter_with_extracted_types() {
        let mut config = LoggingConfig::default();
        config
            .log_types
            .insert("tcp_connection".to_string(), "error".to_string());
        config
            .log_types
            .insert("supervisor".to_string(), "debug".to_string());
        config.exclude_types.push("cache".to_string());

        let filter = InfrarustLogFilter::from_config(&config);

        // Test that the filter was configured correctly
        assert_eq!(
            filter.type_levels.get("tcp_connection"),
            Some(&Level::ERROR)
        );
        assert_eq!(filter.type_levels.get("supervisor"), Some(&Level::DEBUG));
        assert!(filter.excluded_types.contains(&"cache".to_string()));
    }

    #[test]
    fn test_regex_filter_compilation() {
        let mut config = LoggingConfig::default();

        // Test valid regex
        config.regex_filter = Some("error|warn".to_string());
        let filter = InfrarustLogFilter::from_config(&config);
        assert!(filter.regex_filter.is_some());

        // Test invalid regex
        config.regex_filter = Some("[invalid regex".to_string());
        let filter = InfrarustLogFilter::from_config(&config);
        assert!(filter.regex_filter.is_none());

        // Test no regex
        config.regex_filter = None;
        let filter = InfrarustLogFilter::from_config(&config);
        assert!(filter.regex_filter.is_none());
    }

    #[test]
    fn test_regex_filtering_logic() {
        let mut config = LoggingConfig::default();
        config.regex_filter = Some("connection|error".to_string());

        let filter = InfrarustLogFilter::from_config(&config);

        // Test that regex filter exists
        assert!(filter.regex_filter.is_some());
        let regex = filter.regex_filter.as_ref().unwrap();

        // Test regex pattern matching
        assert!(regex.is_match("new connection established"));
        assert!(regex.is_match("fatal error occurred"));
        assert!(!regex.is_match("debug message"));
        assert!(!regex.is_match("info message"));
    }

    #[test]
    fn test_regex_filter_edge_cases() {
        let mut config = LoggingConfig::default();

        // Test empty regex pattern
        config.regex_filter = Some("".to_string());
        let filter = InfrarustLogFilter::from_config(&config);
        assert!(filter.regex_filter.is_some());
        let regex = filter.regex_filter.as_ref().unwrap();
        assert!(regex.is_match("any message")); // Empty regex matches everything

        // Test regex with special characters
        config.regex_filter = Some(r"\berror\b".to_string());
        let filter = InfrarustLogFilter::from_config(&config);
        assert!(filter.regex_filter.is_some());
        let regex = filter.regex_filter.as_ref().unwrap();
        assert!(regex.is_match("an error occurred"));
        assert!(!regex.is_match("errorcode"));

        // Test case-insensitive regex
        config.regex_filter = Some("(?i)ERROR|WARN".to_string());
        let filter = InfrarustLogFilter::from_config(&config);
        assert!(filter.regex_filter.is_some());
        let regex = filter.regex_filter.as_ref().unwrap();
        assert!(regex.is_match("error message"));
        assert!(regex.is_match("ERROR MESSAGE"));
        assert!(regex.is_match("Warning"));
        assert!(!regex.is_match("info"));
    }
}
