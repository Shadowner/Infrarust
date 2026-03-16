//! Tests for InfrarustSampler (feature-gated).
#![cfg(feature = "telemetry")]

use opentelemetry::trace::{SamplingDecision, SpanKind, TraceId};
use opentelemetry_sdk::trace::{Sampler, ShouldSample};

use infrarust_core::telemetry::InfrarustSampler;

fn sample(sampler: &InfrarustSampler, name: &str) -> SamplingDecision {
    sampler
        .should_sample(
            None,
            TraceId::from(1u128),
            name,
            &SpanKind::Server,
            &[],
            &[],
        )
        .decision
}

#[test]
fn test_login_always_sampled() {
    let sampler = InfrarustSampler::new(0.0);
    assert_eq!(
        sample(&sampler, "connection"),
        SamplingDecision::RecordAndSample
    );
}

#[test]
fn test_status_ping_ratio_zero() {
    let sampler = InfrarustSampler::new(0.0);
    assert_eq!(sample(&sampler, "status.ping"), SamplingDecision::Drop);
}

#[test]
fn test_status_ping_ratio_one() {
    let sampler = InfrarustSampler::new(1.0);
    assert_eq!(
        sample(&sampler, "status.ping"),
        SamplingDecision::RecordAndSample
    );
}

#[test]
fn test_orphan_span_sampled() {
    let sampler = InfrarustSampler::new(0.0);
    assert_eq!(
        sample(&sampler, "some.unknown.span"),
        SamplingDecision::RecordAndSample
    );
}

#[test]
fn test_ratio_clamped() {
    // Values outside [0.0, 1.0] should not panic
    let _ = InfrarustSampler::new(-1.0);
    let _ = InfrarustSampler::new(2.0);
}

#[test]
fn test_parent_based_wrapping() {
    // Verify that wrapping in ParentBased works
    let sampler = InfrarustSampler::new(0.5);
    let _parent_based = Sampler::ParentBased(Box::new(sampler));
}
