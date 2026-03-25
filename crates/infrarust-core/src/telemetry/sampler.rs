//! Custom OTel sampler for Infrarust.
//!
//! - Login connections (`"connection"` span) → always sampled (100%)
//! - Status pings (`"status.ping"` span) → ratio-based sampling
//! - Other root spans → always sampled (safe default)

use opentelemetry::KeyValue;
use opentelemetry::trace::{Link, SamplingDecision, SamplingResult, SpanKind, TraceId};
use opentelemetry_sdk::trace::{Sampler, ShouldSample};

/// Custom sampler that always traces login connections and
/// ratio-samples status pings.
#[derive(Debug, Clone)]
pub struct InfrarustSampler {
    status_ratio: f64,
}

impl InfrarustSampler {
    /// `status_ratio` is clamped to `[0.0, 1.0]`.
    pub fn new(status_ratio: f64) -> Self {
        Self {
            status_ratio: status_ratio.clamp(0.0, 1.0),
        }
    }
}

impl ShouldSample for InfrarustSampler {
    fn should_sample(
        &self,
        parent_context: Option<&opentelemetry::Context>,
        trace_id: TraceId,
        name: &str,
        span_kind: &SpanKind,
        attributes: &[KeyValue],
        links: &[Link],
    ) -> SamplingResult {
        match name {
            // Login connections: always trace
            "connection" => SamplingResult {
                decision: SamplingDecision::RecordAndSample,
                attributes: Vec::new(),
                trace_state: Default::default(),
            },
            // Status pings: ratio-based
            "status.ping" => {
                let ratio = Sampler::TraceIdRatioBased(self.status_ratio);
                ratio.should_sample(parent_context, trace_id, name, span_kind, attributes, links)
            }
            // Unknown root spans: always trace (safe default)
            _ => SamplingResult {
                decision: SamplingDecision::RecordAndSample,
                attributes: Vec::new(),
                trace_state: Default::default(),
            },
        }
    }
}
