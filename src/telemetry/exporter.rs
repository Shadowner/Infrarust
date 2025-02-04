use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;

pub fn resource() -> Resource {
    Resource::new(vec![
        KeyValue::new("service.name", env!("CARGO_PKG_NAME")),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
    ])
}