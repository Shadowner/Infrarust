//! OpenTelemetry telemetry module.
//!
//! All OTel code is centralized here. Feature-gated under `telemetry`.
//! Business code uses only `tracing` — never `opentelemetry::*` directly.

#[cfg(feature = "telemetry")]
mod metrics;
#[cfg(feature = "telemetry")]
mod sampler;
#[cfg(feature = "telemetry")]
mod setup;

#[cfg(feature = "telemetry")]
pub use metrics::ProxyMetrics;
#[cfg(feature = "telemetry")]
pub use sampler::InfrarustSampler;
#[cfg(feature = "telemetry")]
pub use setup::{OtelGuard, init_telemetry};
