//! OTel initialization and RAII guard.
//!
//! Follows the official OTel Rust example pattern:
//! - `Resource::builder()` for resource construction
//! - Clone providers before `set_global` (issue #1961)
//! - `eprintln!` in Drop (not `tracing::error!` — subscriber may be dead)

use infrarust_config::TelemetryConfig;
use opentelemetry::global;
use opentelemetry_otlp::{ExporterBuildError, MetricExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};

use crate::error::CoreError;
use crate::telemetry::sampler::InfrarustSampler;

/// Initializes OpenTelemetry providers based on the telemetry config.
///
/// Must be called before the tokio runtime starts handlers, as
/// `ProxyMetrics::new()` uses `global::meter()` which requires
/// the global MeterProvider to be set.
///
/// Returns an `OtelGuard` that flushes and shuts down providers on drop.
pub fn init_telemetry(config: &TelemetryConfig) -> Result<OtelGuard, CoreError> {
    let resource = Resource::builder()
        .with_service_name(config.resource.service_name.clone())
        .build();

    let mut tracer_provider = None;
    let mut meter_provider = None;

    // Traces
    if config.traces.enabled {
        let exporter = build_span_exporter(config)?;
        let sampler = InfrarustSampler::new(config.traces.sampling_ratio);
        let provider = SdkTracerProvider::builder()
            .with_resource(resource.clone())
            .with_batch_exporter(exporter)
            .with_sampler(Sampler::ParentBased(Box::new(sampler)))
            .build();
        // Clone before set_global — issue OTel #1961
        global::set_tracer_provider(provider.clone());
        tracer_provider = Some(provider);
    }

    // Metrics
    if config.metrics.enabled {
        let exporter = build_metric_exporter(config)?;
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter)
            .with_resource(resource)
            .build();
        // Clone before set_global
        global::set_meter_provider(provider.clone());
        meter_provider = Some(provider);
    }

    Ok(OtelGuard {
        tracer_provider,
        meter_provider,
    })
}

/// RAII guard that shuts down OTel providers on drop.
///
/// Must remain in scope for the entire lifetime of `main()`.
pub struct OtelGuard {
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        // Use eprintln!, NOT tracing::error! — the tracing subscriber
        // may already be destroyed at this point.
        if let Some(tp) = self.tracer_provider.take()
            && let Err(e) = tp.shutdown()
        {
            eprintln!("OpenTelemetry tracer shutdown error: {e}");
        }
        if let Some(mp) = self.meter_provider.take()
            && let Err(e) = mp.shutdown()
        {
            eprintln!("OpenTelemetry meter shutdown error: {e}");
        }
    }
}

/// Builds a span exporter based on the configured protocol.
fn build_span_exporter(config: &TelemetryConfig) -> Result<SpanExporter, CoreError> {
    let endpoint = config
        .endpoint
        .as_deref()
        .unwrap_or("http://localhost:4317");

    match config.protocol.as_str() {
        "grpc" => SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build()
            .map_err(|e: ExporterBuildError| CoreError::TelemetryInit(e.to_string())),
        "http" => SpanExporter::builder()
            .with_http()
            .with_endpoint(endpoint)
            .build()
            .map_err(|e: ExporterBuildError| CoreError::TelemetryInit(e.to_string())),
        other => Err(CoreError::TelemetryInit(format!(
            "unsupported protocol: {other}, expected 'grpc' or 'http'"
        ))),
    }
}

/// Builds a metric exporter based on the configured protocol.
fn build_metric_exporter(config: &TelemetryConfig) -> Result<MetricExporter, CoreError> {
    let endpoint = config
        .endpoint
        .as_deref()
        .unwrap_or("http://localhost:4317");

    match config.protocol.as_str() {
        "grpc" => MetricExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build()
            .map_err(|e: ExporterBuildError| CoreError::TelemetryInit(e.to_string())),
        "http" => MetricExporter::builder()
            .with_http()
            .with_endpoint(endpoint)
            .build()
            .map_err(|e: ExporterBuildError| CoreError::TelemetryInit(e.to_string())),
        other => Err(CoreError::TelemetryInit(format!(
            "unsupported protocol: {other}, expected 'grpc' or 'http'"
        ))),
    }
}
