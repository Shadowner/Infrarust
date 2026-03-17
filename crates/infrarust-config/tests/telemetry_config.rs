#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Tests for the extended `TelemetryConfig`.

use std::time::Duration;

use infrarust_config::{MetricsConfig, ProxyConfig, ResourceConfig, TelemetryConfig, TracesConfig};

#[test]
fn test_telemetry_config_default() {
    let tc = TelemetryConfig::default();
    assert!(!tc.enabled);
    assert!(tc.endpoint.is_none());
    assert_eq!(tc.protocol, "grpc");
    assert!(tc.metrics.enabled);
    assert!(tc.traces.enabled);
    assert_eq!(tc.resource.service_name, "infrarust");
}

#[test]
fn test_telemetry_config_parse_full() {
    let toml_str = r#"
        [telemetry]
        enabled = true
        endpoint = "http://otel:4317"
        protocol = "http"

        [telemetry.metrics]
        enabled = false
        export_interval = "30s"

        [telemetry.traces]
        enabled = true
        sampling_ratio = 0.5

        [telemetry.resource]
        service_name = "my-proxy"
        service_version = "1.0.0"
    "#;

    // Parse as a minimal ProxyConfig wrapper
    let config: ProxyConfig = toml::from_str(toml_str).expect("should parse");
    let tc = config.telemetry.expect("telemetry should be Some");
    assert!(tc.enabled);
    assert_eq!(tc.endpoint.as_deref(), Some("http://otel:4317"));
    assert_eq!(tc.protocol, "http");
    assert!(!tc.metrics.enabled);
    assert_eq!(tc.metrics.export_interval, Duration::from_secs(30));
    assert!(tc.traces.enabled);
    assert!((tc.traces.sampling_ratio - 0.5).abs() < f64::EPSILON);
    assert_eq!(tc.resource.service_name, "my-proxy");
    assert_eq!(tc.resource.service_version, "1.0.0");
}

#[test]
fn test_telemetry_config_absent() {
    let config: ProxyConfig = toml::from_str("").expect("should parse");
    assert!(config.telemetry.is_none());
}

#[test]
fn test_telemetry_config_disabled() {
    let toml_str = r"
        [telemetry]
        enabled = false
    ";
    let config: ProxyConfig = toml::from_str(toml_str).expect("should parse");
    let tc = config.telemetry.expect("telemetry should be Some");
    assert!(!tc.enabled);
}

#[test]
fn test_metrics_config_default() {
    let mc = MetricsConfig::default();
    assert!(mc.enabled);
    assert_eq!(mc.export_interval, Duration::from_secs(15));
}

#[test]
fn test_traces_config_default() {
    let tc = TracesConfig::default();
    assert!(tc.enabled);
    assert!((tc.sampling_ratio - 0.1).abs() < f64::EPSILON);
}

#[test]
fn test_resource_config_default() {
    let rc = ResourceConfig::default();
    assert_eq!(rc.service_name, "infrarust");
    // service_version is env!("CARGO_PKG_VERSION"), just check it's not empty
    assert!(!rc.service_version.is_empty());
}
