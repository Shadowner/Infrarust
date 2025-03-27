use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;

pub fn resource() -> Resource {
    opentelemetry_sdk::Resource::builder()
        .with_service_name(env!("CARGO_PKG_NAME"))
        .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
        .build()
}
