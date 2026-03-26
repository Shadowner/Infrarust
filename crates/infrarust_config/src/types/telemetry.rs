//! OpenTelemetry configuration.

use std::time::Duration;

use serde::Deserialize;

use crate::defaults;

/// Sub-sections: `[telemetry.metrics]`, `[telemetry.traces]`, `[telemetry.resource]`.
/// Absent from the TOML file means `None` in `ProxyConfig` (no telemetry).
#[derive(Debug, Clone, Deserialize)]
pub struct TelemetryConfig {
    /// Enables telemetry. `false` = initialized but no export.
    #[serde(default)]
    pub enabled: bool,

    /// Endpoint OTLP (ex: "<http://localhost:4317>"). `None` = SDK default.
    #[serde(default)]
    pub endpoint: Option<String>,

    /// Export protocol: "grpc" or "http".
    #[serde(default = "defaults::telemetry_protocol")]
    pub protocol: String,

    /// Metrics configuration.
    #[serde(default)]
    pub metrics: MetricsConfig,

    /// Traces configuration.
    #[serde(default)]
    pub traces: TracesConfig,

    /// `OTel` resource attributes.
    #[serde(default)]
    pub resource: ResourceConfig,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            protocol: defaults::telemetry_protocol(),
            metrics: MetricsConfig::default(),
            traces: TracesConfig::default(),
            resource: ResourceConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsConfig {
    /// Enables metrics export.
    #[serde(default = "defaults::true_val")]
    pub enabled: bool,

    /// Metrics export interval.
    #[serde(default = "defaults::metrics_export_interval")]
    #[serde(with = "humantime_serde")]
    pub export_interval: Duration,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::true_val(),
            export_interval: defaults::metrics_export_interval(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TracesConfig {
    /// Enables traces export.
    #[serde(default = "defaults::true_val")]
    pub enabled: bool,

    /// Sampling ratio for status pings (0.0-1.0).
    /// Login connections are always traced at 100%.
    #[serde(default = "defaults::sampling_ratio")]
    pub sampling_ratio: f64,
}

impl Default for TracesConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::true_val(),
            sampling_ratio: defaults::sampling_ratio(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceConfig {
    /// `OTel` service name.
    #[serde(default = "defaults::service_name")]
    pub service_name: String,

    /// `OTel` service version.
    #[serde(default = "defaults::service_version")]
    pub service_version: String,
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            service_name: defaults::service_name(),
            service_version: defaults::service_version(),
        }
    }
}
