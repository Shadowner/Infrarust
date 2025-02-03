use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;

pub fn resource() -> Resource {
    Resource::new(vec![
        KeyValue::new("service.name", env!("CARGO_PKG_NAME")),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
    ])
}

pub fn configure_otlp_exporter() -> opentelemetry_otlp::ExportConfig {
    opentelemetry_otlp::ExportConfig {
        endpoint: Some("http://127.0.0.1:4317".to_string()),
        timeout: std::time::Duration::from_secs(1),
        ..Default::default()
    }
}