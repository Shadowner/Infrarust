use serde::Deserialize;

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
