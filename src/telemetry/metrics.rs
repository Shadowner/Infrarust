use opentelemetry::metrics::{Counter, Gauge, Histogram, UpDownCounter};
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    metrics::{MeterProviderBuilder, PeriodicReader, SdkMeterProvider},
    runtime, Resource,
};
use std::io::Error;
use std::time::{Duration, Instant};
use uuid::Uuid;

use super::Direction;

pub struct MeterProviderGuard(pub SdkMeterProvider);

pub fn init_meter_provider(
    resource: Resource,
    endpoint: String,
    duration: Duration,
) -> MeterProviderGuard {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .unwrap();

    let reader = PeriodicReader::builder(exporter, runtime::Tokio)
        .with_interval(duration)
        .build();

    let provider = MeterProviderBuilder::default()
        .with_reader(reader)
        .with_resource(resource)
        .build();

    global::set_meter_provider(provider.clone());

    MeterProviderGuard(provider)
}

pub struct InfrarustMetrics {
    // Connexions
    pub active_connections: UpDownCounter<i64>,
    pub connection_errors: Counter<u64>,
    pub bytes_transferred: Gauge<u64>,
    pub connection_latency: Histogram<f64>,
    pub requests_per_second: Counter<u64>,
    pub tota_bytes_transferred: Counter<u64>,

    // Backend
    pub active_backends: UpDownCounter<i64>,
    pub backend_latency: Histogram<f64>,
    pub backend_errors: Counter<u64>,
    pub backend_requests: Counter<u64>,

    // Système
    pub cpu_usage: Histogram<f64>,
    pub memory_usage: Histogram<f64>,
    pub open_files: UpDownCounter<i64>,
    pub thread_count: UpDownCounter<i64>,
    pub internal_errors: Counter<u64>,

    // Minecraft spécifique
    pub protocol_errors: Counter<u64>,
    pub player_count: UpDownCounter<i64>,
    pub packet_processing_time: Histogram<f64>,
}

impl Default for InfrarustMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl InfrarustMetrics {
    pub fn new() -> Self {
        let meter = global::meter("infrarust");

        Self {
            // Connexions
            active_connections: meter
                .i64_up_down_counter("connections.active")
                .with_description("Current number of active connections")
                .with_unit("connections")
                .build(),
            connection_errors: meter
                .u64_counter("connections.errors")
                .with_description("Number of connection errors")
                .with_unit("errors")
                .build(),
            bytes_transferred: meter
                .u64_gauge("network.bytes")
                .with_description("Total bytes transferred")
                .with_unit("bytes")
                .build(),
            tota_bytes_transferred: meter
                .u64_counter("network.bytes.total")
                .with_description("Total bytes transferred since start")
                .with_unit("bytes")
                .build(),
            connection_latency: meter
                .f64_histogram("connections.latency")
                .with_description("Connection latency")
                .with_unit("ms")
                .build(),
            requests_per_second: meter
                .u64_counter("requests.rate")
                .with_description("Number of requests per second")
                .with_unit("requests")
                .build(),

            // Backend
            active_backends: meter
                .i64_up_down_counter("backends.active")
                .with_description("Number of active backend servers")
                .with_unit("servers")
                .build(),
            backend_latency: meter
                .f64_histogram("backends.latency")
                .with_description("Backend server response time")
                .with_unit("ms")
                .build(),
            backend_errors: meter
                .u64_counter("backends.errors")
                .with_description("Number of backend errors")
                .with_unit("errors")
                .build(),
            backend_requests: meter
                .u64_counter("backends.requests")
                .with_description("Total backend requests")
                .with_unit("requests")
                .build(),

            // Système
            cpu_usage: meter
                .f64_histogram("system.cpu")
                .with_description("CPU usage percentage")
                .with_unit("percent")
                .build(),
            memory_usage: meter
                .f64_histogram("system.memory")
                .with_description("Memory usage")
                .with_unit("bytes")
                .build(),
            open_files: meter
                .i64_up_down_counter("system.open_files")
                .with_description("Number of open files")
                .with_unit("files")
                .build(),
            thread_count: meter
                .i64_up_down_counter("system.threads")
                .with_description("Number of threads")
                .with_unit("threads")
                .build(),
            internal_errors: meter
                .u64_counter("system.internal_errors")
                .with_description("Number of internal errors")
                .with_unit("errors")
                .build(),

            // Minecraft spécifique
            protocol_errors: meter
                .u64_counter("minecraft.protocol_errors")
                .with_description("Number of Minecraft protocol errors")
                .with_unit("errors")
                .build(),
            player_count: meter
                .i64_up_down_counter("minecraft.players")
                .with_description("Number of connected players")
                .with_unit("players")
                .build(),
            packet_processing_time: meter
                .f64_histogram("minecraft.packet_time")
                .with_description("Packet processing time")
                .with_unit("ms")
                .build(),
        }
    }

    pub fn record_new_connection(&self, client_ip: &str, hostname: &str, session_id: Uuid) {
        self.active_connections.add(
            1,
            &[
                KeyValue::new("client_ip", client_ip.to_string()),
                KeyValue::new("hostname", hostname.to_string()),
                KeyValue::new("session_id", session_id.to_string()),
            ],
        );
    }

    pub fn record_connection_end(&self, client_ip: &str, reason: &str, session_id: Uuid) {
        self.active_connections.add(
            -1,
            &[
                KeyValue::new("client_ip", client_ip.to_string()),
                KeyValue::new("reason", reason.to_string()),
                KeyValue::new("session_id", session_id.to_string()),
            ],
        );
    }

    // Méthodes pour les connexions
    pub fn record_connection_error(&self, error_type: &str, session_id: Uuid) {
        self.connection_errors.add(
            1,
            &[
                KeyValue::new("error_type", error_type.to_string()),
                KeyValue::new("session_id", session_id.to_string()),
            ],
        );
    }

    pub fn record_bytes_transferred(&self, direction: Direction, bytes: u64, session_id: Uuid) {
        let direction_str = match direction {
            Direction::Incoming => "incoming",
            Direction::Outgoing => "outgoing",
            Direction::Internal => "internal (code)",
        };

        self.bytes_transferred.record(
            bytes,
            &[
                KeyValue::new("direction", direction_str.to_string()),
                KeyValue::new("session_id", session_id.to_string()),
            ],
        );

        self.tota_bytes_transferred.add(
            bytes,
            &[
                KeyValue::new("direction", direction_str.to_string()),
                KeyValue::new("session_id", session_id.to_string()),
            ],
        );
    }

    pub fn record_request(&self) {
        self.requests_per_second.add(1, &[]);
    }

    // Méthodes pour les backends
    pub fn record_backend_request_start(
        &self,
        config_id: &str,
        server: &str,
        session_id: &Uuid,
    ) -> Instant {
        self.backend_requests.add(
            1,
            &[
                KeyValue::new("config_id", config_id.to_string()),
                KeyValue::new("server", server.to_string()),
                KeyValue::new("session_id", session_id.to_string()),
            ],
        );
        Instant::now()
    }

    pub fn record_backend_request_end(
        &self,
        config_id: &str,
        server: &str,
        start_time: Instant,
        success: bool,
        session_id: &Uuid,
        error: Option<&Error>,
    ) {
        let duration = start_time.elapsed().as_secs_f64() * 1000.0;
        self.backend_latency.record(
            duration,
            &[
                KeyValue::new("config_id", config_id.to_string()),
                KeyValue::new("server", server.to_string()),
                KeyValue::new("success", success.to_string()),
                KeyValue::new("session_id", session_id.to_string()),
            ],
        );

        if !success && error.is_some() {
            self.backend_errors.add(
                1,
                &[
                    KeyValue::new("config_id", config_id.to_string()),
                    KeyValue::new("server", server.to_string()),
                    KeyValue::new("session_id", session_id.to_string()),
                    KeyValue::new("error", error.as_ref().map(|e| e.to_string()).unwrap()),
                ],
            );
        }
    }

    pub fn record_backend_response(
        &self,
        server: &str,
        response: &str,
        duration: Duration,
        status_code: u16,
        is_error: bool,
        session_id: Uuid,
    ) {
        let duration_ms = duration.as_secs_f64() * 1000.0;

        self.backend_latency.record(
            duration_ms,
            &[
                KeyValue::new("server", server.to_string()),
                KeyValue::new("response_type", response.to_string()),
                KeyValue::new("status", status_code.to_string()),
                KeyValue::new("session_id", session_id.to_string()),
            ],
        );

        if is_error {
            self.backend_errors.add(
                1,
                &[
                    KeyValue::new("server", server.to_string()),
                    KeyValue::new("error_type", response.to_string()),
                    KeyValue::new("status_code", status_code.to_string()),
                    KeyValue::new("session_id", session_id.to_string()),
                ],
            );
        }

        // Mettre à jour le compteur de requêtes
        self.backend_requests.add(
            1,
            &[
                KeyValue::new("server", server.to_string()),
                KeyValue::new("success", (!is_error).to_string()),
                KeyValue::new("session_id", session_id.to_string()),
            ],
        );
    }

    pub fn update_backend_count(&self, delta: i64, server: &str) {
        self.active_backends
            .add(delta, &[KeyValue::new("server", server.to_string())]);
    }

    // Méthodes système
    pub fn update_system_metrics(&self, cpu: f64, memory: f64, threads: i64) {
        self.cpu_usage.record(cpu, &[]);
        self.memory_usage.record(memory, &[]);
        self.thread_count.add(threads, &[]);
    }

    pub fn record_internal_error(
        &self,
        error_type: &str,
        error: Option<&Error>,
        trace_id: Option<Uuid>,
    ) {
        self.internal_errors.add(
            1,
            &[
                KeyValue::new("error_type", error_type.to_string()),
                KeyValue::new("error", error.map(|e| e.to_string()).unwrap_or_default()),
                KeyValue::new(
                    "trace_id",
                    trace_id.map(|id| id.to_string()).unwrap_or_default(),
                ),
            ],
        );
    }

    // Méthodes Minecraft
    pub fn record_protocol_error(&self, error_type: &str, details: &str, session_id: Uuid) {
        self.protocol_errors.add(
            1,
            &[
                KeyValue::new("error_type", error_type.to_string()),
                KeyValue::new("details", details.to_string()),
                KeyValue::new("session_id", session_id.to_string()),
            ],
        );
    }

    pub fn update_player_count(&self, delta: i64, server: &str, session_id: Uuid, username: &str) {
        self.player_count.add(
            delta,
            &[
                KeyValue::new("server", server.to_string()),
                KeyValue::new("session_id", session_id.to_string()),
                KeyValue::new("username", username.to_string()),
            ],
        );
    }

    pub fn record_packet_processing(&self, packet_type: &str, duration_ms: f64, session_id: Uuid) {
        self.packet_processing_time.record(
            duration_ms,
            &[
                KeyValue::new("packet_type", packet_type.to_string()),
                KeyValue::new("session_id", session_id.to_string()),
            ],
        );
    }
}
