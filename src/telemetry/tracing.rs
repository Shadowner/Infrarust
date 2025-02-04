use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    trace::{RandomIdGenerator, Sampler, Tracer, TracerProvider},
    Resource,
};
use tracing::Level;
use tracing_subscriber::{fmt::format::FmtSpan, Layer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use std::str::FromStr;

use crate::CONFIG;

pub struct TracerProviderGuard(pub TracerProvider);

impl Drop for TracerProviderGuard {
    fn drop(&mut self) {
        global::shutdown_tracer_provider();
    }
}

pub fn init_tracer_provider(
    resource: Resource,
    export_url: Option<String>,
) -> TracerProviderGuard {
    let mut provider = TracerProvider::builder();
    if export_url.clone().is_some() {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(export_url.clone().unwrap().to_string())
            .build()
            .unwrap();

        provider = provider.with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio);
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

pub fn init_subscriber_with_optl(provider: &TracerProvider) {
    let tracer = provider.tracer("proxy");
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let log_level = if cfg!(debug_assertions) {
        Level::DEBUG
    } else {
        Level::INFO
    };

    let fmt_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_level(true)           // Garder le niveau de log
        .with_filter(tracing_subscriber::filter::LevelFilter::from_level(log_level));

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_str(&format!("infrarust={}", log_level))
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::from_default_env()))
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
        .with_filter(tracing_subscriber::filter::LevelFilter::from_level(log_level));

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_str(&format!("infrarust={}", log_level))
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::from_default_env()))
        .with(fmt_layer)
        .init();
}
