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
}

impl Default for LoggingConfig {
    fn default() -> Self {
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
        }
    }
}
