#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Tests for `ProxyMetrics` (feature-gated).
#![cfg(feature = "telemetry")]

use infrarust_core::telemetry::ProxyMetrics;

#[test]
fn test_metrics_construction() {
    // ProxyMetrics::new() should not panic even without a real collector
    let _metrics = ProxyMetrics::new();
}

#[test]
fn test_counter_increment() {
    let metrics = ProxyMetrics::new();
    metrics.record_connection_start("test-server", "passthrough");
}

#[test]
fn test_gauge_up_down() {
    let metrics = ProxyMetrics::new();
    metrics.record_player_join("test-server");
    metrics.record_player_leave("test-server");
}

#[test]
fn test_histogram_record() {
    let metrics = ProxyMetrics::new();
    metrics.record_connection_end(1.5, "test-server", "passthrough");
    metrics.record_backend_connect(0.05, "test-server");
    metrics.record_handshake(0.01);
}

#[test]
fn test_rejection_counter() {
    let metrics = ProxyMetrics::new();
    metrics.record_rejection("rate_limit");
    metrics.record_rejection("ban");
}

#[test]
fn test_packets_relayed() {
    let metrics = ProxyMetrics::new();
    metrics.record_packets_relayed(100, "c2s");
    metrics.record_packets_relayed(200, "s2c");
}
