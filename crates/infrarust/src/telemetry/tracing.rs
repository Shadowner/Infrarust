use std::str::FromStr;

use infrarust_config::LoggingConfig;
use infrarust_config::TelemetryConfig;
use tracing::Level;
use tracing_subscriber::Layer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use super::infrarust_fmt_formatter::InfrarustMessageFormatter;
use super::log_filter::InfrarustLogFilter;
use super::log_type_layer::LogTypeLayer;

#[cfg(feature = "telemetry")]
use opentelemetry::global;
#[cfg(feature = "telemetry")]
use opentelemetry::trace::TracerProvider;
#[cfg(feature = "telemetry")]
use opentelemetry_otlp::WithExportConfig;
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::Resource;
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::trace::{SdkTracerProvider, TracerProviderBuilder};

#[cfg(feature = "telemetry")]
pub struct TracerProviderGuard(pub SdkTracerProvider);

#[cfg(feature = "telemetry")]
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
/// This sets up the tracing subscriber with custom formatting and log type extraction.
/// It should be called once at application startup before any logging occurs.
pub fn init_logging(config: &LoggingConfig) -> LoggingGuard {
    let log_level = if cfg!(debug_assertions) || config.debug {
        Level::DEBUG
    } else {
        Level::INFO
    };

    let formatter = create_formatter_from_config(config);

    let log_type_layer = LogTypeLayer::new();
    let storage = log_type_layer.storage().clone();

    let infrarust_filter = InfrarustLogFilter::from_config(config).with_log_type_storage(storage);

    let env_filter = tracing_subscriber::EnvFilter::from_str(&format!("infrarust={}", log_level))
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::from_default_env());

    let has_regex_filter = infrarust_filter.get_regex_filter().is_some();

    if has_regex_filter {
        let regex_layer = infrarust_filter.create_regex_layer().unwrap();
        let fmt_layer = tracing_subscriber::fmt::layer().event_format(formatter);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(log_type_layer)
            .with(regex_layer)
            .with(fmt_layer)
            .init();
    } else {
        let fmt_layer = tracing_subscriber::fmt::layer()
            .event_format(formatter)
            .with_filter(tracing_subscriber::filter::LevelFilter::from_level(
                log_level,
            ))
            .with_filter(infrarust_filter.create_filter_fn());

        tracing_subscriber::registry()
            .with(env_filter)
            .with(log_type_layer)
            .with(fmt_layer)
            .init();
    }

    LoggingGuard {}
}

/// Initialize OpenTelemetry tracing
///
/// This sets up an OpenTelemetry tracer provider for distributed tracing
/// and adds a layer to the existing tracing subscriber.
#[cfg(feature = "telemetry")]
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

#[cfg(not(feature = "telemetry"))]
pub fn init_opentelemetry_tracing(_resource: (), _config: &TelemetryConfig) -> Option<()> {
    None
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
