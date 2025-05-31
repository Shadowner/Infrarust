use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LoggingConfig {
    #[serde(default)]
    pub use_color: bool,

    #[serde(default)]
    pub debug: bool,

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

    #[serde(default)]
    pub log_types: HashMap<String, String>,

    #[serde(default)]
    pub exclude_types: Vec<String>,

    /// Global minimum log level (overrides type-specific levels if higher)
    #[serde(default)]
    pub min_level: Option<String>,

    pub regex_filter: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogType {
    TcpConnection,
    Supervisor,
    ServerManager,
    PacketProcessing,
    BanSystem,
    Authentication,
    Telemetry,
    ConfigProvider,
    ProxyProtocol,
    ProxyMode,
    Cache,
    Filter,
    Motd,
    Custom(String),
}

impl LogType {
    pub fn as_str(&self) -> &str {
        match self {
            LogType::TcpConnection => "tcp_connection",
            LogType::Supervisor => "supervisor",
            LogType::ServerManager => "server_manager",
            LogType::PacketProcessing => "packet_processing",
            LogType::BanSystem => "ban_system",
            LogType::Authentication => "authentication",
            LogType::Telemetry => "telemetry",
            LogType::ConfigProvider => "config_provider",
            LogType::ProxyProtocol => "proxy_protocol",
            LogType::Cache => "cache",
            LogType::Filter => "filter",
            LogType::Motd => "motd",
            LogType::ProxyMode => "proxy_mode",
            LogType::Custom(name) => name,
        }
    }
}

impl From<&str> for LogType {
    fn from(s: &str) -> Self {
        match s {
            "tcp_connection" => LogType::TcpConnection,
            "supervisor" => LogType::Supervisor,
            "server_manager" => LogType::ServerManager,
            "packet_processing" => LogType::PacketProcessing,
            "ban_system" => LogType::BanSystem,
            "authentication" => LogType::Authentication,
            "telemetry" => LogType::Telemetry,
            "config_provider" => LogType::ConfigProvider,
            "proxy_protocol" => LogType::ProxyProtocol,
            "cache" => LogType::Cache,
            "filter" => LogType::Filter,
            "motd" => LogType::Motd,
            "proxy_mode" => LogType::ProxyMode,
            other => LogType::Custom(other.to_string()),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        let mut default_log_types = HashMap::new();
        default_log_types.insert("tcp_connection".to_string(), "info".to_string());
        default_log_types.insert("supervisor".to_string(), "debug".to_string());
        default_log_types.insert("server_manager".to_string(), "info".to_string());
        default_log_types.insert("packet_processing".to_string(), "debug".to_string());
        default_log_types.insert("ban_system".to_string(), "info".to_string());
        default_log_types.insert("authentication".to_string(), "info".to_string());
        default_log_types.insert("telemetry".to_string(), "warn".to_string());
        default_log_types.insert("config_provider".to_string(), "info".to_string());
        default_log_types.insert("proxy_protocol".to_string(), "debug".to_string());
        default_log_types.insert("cache".to_string(), "debug".to_string());
        default_log_types.insert("filter".to_string(), "info".to_string());
        default_log_types.insert("proxy_mode".to_string(), "info".to_string());
        default_log_types.insert("motd".to_string(), "debug".to_string());

        Self {
            debug: false,
            use_color: true,
            use_icons: true,
            show_timestamp: true,
            time_format: "%Y-%m-%d %H:%M:%S%.3f".to_string(),
            show_target: false,
            show_fields: false,
            template: "{timestamp} {level}: {message}".to_string(),
            field_prefixes: HashMap::new(),
            log_types: default_log_types,
            exclude_types: Vec::new(),
            min_level: None,
            regex_filter: None,
        }
    }
}
