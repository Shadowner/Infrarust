//! OTel metrics instruments for the proxy.
//!
//! All metric recording happens through helper methods, keeping
//! `opentelemetry::KeyValue` imports out of handler code.

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram, UpDownCounter};

/// Central collection of proxy metric instruments.
///
/// Created once after `init_telemetry()` sets the global MeterProvider.
/// Passed to handlers as `Option<Arc<ProxyMetrics>>`.
pub struct ProxyMetrics {
    connections_total: Counter<u64>,
    connections_active: UpDownCounter<i64>,
    connections_rejected: Counter<u64>,
    players_online: UpDownCounter<i64>,
    connection_duration: Histogram<f64>,
    handshake_duration: Histogram<f64>,
    backend_connect_duration: Histogram<f64>,
    packets_relayed: Counter<u64>,
}

impl Default for ProxyMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl ProxyMetrics {
    /// Creates all instruments from the global meter provider.
    pub fn new() -> Self {
        let meter = opentelemetry::global::meter("infrarust");
        Self {
            connections_total: meter
                .u64_counter("infrarust.connections.total")
                .with_description("Total number of connections")
                .build(),
            connections_active: meter
                .i64_up_down_counter("infrarust.connections.active")
                .with_description("Currently active connections")
                .build(),
            connections_rejected: meter
                .u64_counter("infrarust.connections.rejected")
                .with_description("Rejected connections")
                .build(),
            players_online: meter
                .i64_up_down_counter("infrarust.players.online")
                .with_description("Players currently connected")
                .build(),
            connection_duration: meter
                .f64_histogram("infrarust.connection.duration")
                .with_description("Connection duration")
                .with_unit("s")
                .build(),
            handshake_duration: meter
                .f64_histogram("infrarust.handshake.duration")
                .with_description("Handshake processing duration")
                .with_unit("s")
                .build(),
            backend_connect_duration: meter
                .f64_histogram("infrarust.backend.connect.duration")
                .with_description("Backend connection time")
                .with_unit("s")
                .build(),
            packets_relayed: meter
                .u64_counter("infrarust.packets.relayed")
                .with_description("Total packets relayed")
                .build(),
        }
    }

    // ── Helper methods ── keep KeyValue out of handler code ──

    /// Records the start of a new connection.
    pub fn record_connection_start(&self, server: &str, mode: &str) {
        self.connections_total.add(
            1,
            &[
                KeyValue::new("server", server.to_string()),
                KeyValue::new("proxy_mode", mode.to_string()),
            ],
        );
        self.connections_active.add(1, &[]);
    }

    /// Records the end of a connection with its duration.
    pub fn record_connection_end(&self, duration_secs: f64, server: &str, mode: &str) {
        self.connections_active.add(-1, &[]);
        self.connection_duration.record(
            duration_secs,
            &[
                KeyValue::new("server", server.to_string()),
                KeyValue::new("proxy_mode", mode.to_string()),
            ],
        );
    }

    /// Records a player joining.
    pub fn record_player_join(&self, server: &str) {
        self.players_online
            .add(1, &[KeyValue::new("server", server.to_string())]);
    }

    /// Records a player leaving.
    pub fn record_player_leave(&self, server: &str) {
        self.players_online
            .add(-1, &[KeyValue::new("server", server.to_string())]);
    }

    /// Records a rejected connection.
    pub fn record_rejection(&self, reason: &str) {
        self.connections_rejected
            .add(1, &[KeyValue::new("reason", reason.to_string())]);
    }

    /// Records backend connection duration.
    pub fn record_backend_connect(&self, duration_secs: f64, server: &str) {
        self.backend_connect_duration.record(
            duration_secs,
            &[KeyValue::new("server", server.to_string())],
        );
    }

    /// Records handshake processing duration.
    pub fn record_handshake(&self, duration_secs: f64) {
        self.handshake_duration.record(duration_secs, &[]);
    }

    /// Records relayed packets.
    pub fn record_packets_relayed(&self, count: u64, direction: &str) {
        self.packets_relayed
            .add(count, &[KeyValue::new("direction", direction.to_string())]);
    }
}
