use std::str::FromStr;

use opentelemetry::global;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{SdkTracerProvider, TracerProviderBuilder};
use tracing::Level;
use tracing_subscriber::Layer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::core::config::{LoggingConfig, TelemetryConfig};

use super::infrarust_fmt_formatter::InfrarustMessageFormatter;

pub struct TracerProviderGuard(pub SdkTracerProvider);
impl Drop for TracerProviderGuard {
    fn drop(&mut self) {
        if let Err(e) = self.0.shutdown() {
            eprintln!("Failed to shutdown OpenTelemetry exporter: {:?}", e);
        };
    }
}

pub struct LoggingGuard;

/// Initialize logging with the provided configuration
///
/// This sets up the tracing subscriber with custom formatting based on the config.
/// It should be called once at application startup before any logging occurs.
pub fn init_logging(config: &LoggingConfig) -> LoggingGuard {
    let log_level = if cfg!(debug_assertions) {
        Level::DEBUG
    } else {
        Level::INFO
    };

    let formatter = create_formatter_from_config(config);

    let fmt_layer = tracing_subscriber::fmt::layer()
        .event_format(formatter)
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

    LoggingGuard {}
}

/// Initialize OpenTelemetry tracing
///
/// This sets up an OpenTelemetry tracer provider for distributed tracing
/// and adds a layer to the existing tracing subscriber.
pub fn init_opentelemetry_tracing(
    resource: Resource,
    config: &TelemetryConfig,
) -> Option<TracerProviderGuard> {
    let export_url = match &config.export_url {
        Some(url) if !url.is_empty() => url.clone(),
        _ => return None,
    };

    let exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(export_url)
        .build()
    {
        Ok(exporter) => exporter,
        Err(e) => {
            tracing::error!("Failed to create OpenTelemetry exporter: {}", e);
            return None;
        }
    };

    let provider = TracerProviderBuilder::default()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();

    let tracer = provider.tracer("infrarust");
    let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    // Try to add the telemetry layer to the registry
    // It's ok if this fails - it means a subscriber is already registered
    // and we'll just add the layer to it
    let _ = tracing_subscriber::registry()
        .with(telemetry_layer)
        .try_init();

    global::set_tracer_provider(provider.clone());

    Some(TracerProviderGuard(provider))
}

fn create_formatter_from_config(config: &LoggingConfig) -> InfrarustMessageFormatter {
    let mut formatter = InfrarustMessageFormatter::default()
        .with_ansi(config.use_color)
        .with_icons(config.use_icons)
        .with_timestamp(config.show_timestamp)
        .with_time_format(&config.time_format)
        .with_target(config.show_target)
        .with_all_fields(config.show_fields);

    if !config.template.is_empty() {
        formatter = formatter.with_template(&config.template);
    }

    for (field, prefix) in &config.field_prefixes {
        formatter = formatter.before_field(field, prefix);
    }

    formatter
}
