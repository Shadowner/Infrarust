use opentelemetry::global;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::{SdkTracerProvider, TracerProviderBuilder};
use opentelemetry_sdk::Resource;
use std::str::FromStr;
use tracing::Level;
use tracing_subscriber::Layer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub struct TracerProviderGuard(pub SdkTracerProvider);

impl Drop for TracerProviderGuard {
    fn drop(&mut self) {
        if let Err(exporter) = self.0.shutdown() {
            println!("Failed to shutdown exporter: {:?}", exporter);
        };
    }
}

pub fn init_tracer_provider(resource: Resource, export_url: Option<String>) -> TracerProviderGuard {
    let mut provider = TracerProviderBuilder::default();
    if export_url.clone().is_some() {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(export_url.clone().unwrap().to_string())
            .build()
            .unwrap();

        provider = provider.with_batch_exporter(exporter);
    }

    provider = provider.with_resource(resource);

    let provider = provider.build();

    if export_url.is_some() {
        init_subscriber_with_optl(&provider);
    } else {
        init_subscriber();
    }
    global::set_tracer_provider(provider.clone());
    TracerProviderGuard(provider)
}

pub fn init_subscriber_with_optl(provider: &SdkTracerProvider) {
    let tracer = provider.tracer("proxy");
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let log_level = if cfg!(debug_assertions) {
        Level::DEBUG
    } else {
        Level::INFO
    };

    let fmt_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_level(true) // Garder le niveau de log
        .with_filter(tracing_subscriber::filter::LevelFilter::from_level(
            log_level,
        ));

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::from_str(&format!("infrarust={}", log_level))
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::from_default_env()),
        )
        .with(layer)
        .with(fmt_layer)
        .init();
}

pub fn init_subscriber() {
    let log_level = if cfg!(debug_assertions) {
        Level::DEBUG
    } else {
        Level::INFO
    };

    let fmt_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_level(true)
        .with_filter(tracing_subscriber::filter::LevelFilter::from_level(
            log_level,
        ));

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::from_str(&format!("infrarust={}", log_level))
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::from_default_env()),
        )
        .with(fmt_layer)
        .init();
}
