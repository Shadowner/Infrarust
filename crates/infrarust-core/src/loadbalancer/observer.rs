//! Fan-out of backend connect outcomes to health and telemetry.

use std::sync::Arc;

use infrarust_transport::{ConnectAttempt, ConnectAttemptObserver};

use super::PassiveBackendHealth;
#[cfg(feature = "telemetry")]
use super::{BackendState, HealthTransitionListener};

pub struct BackendAttemptObserver {
    health: Arc<PassiveBackendHealth>,
    #[cfg(feature = "telemetry")]
    metrics: Arc<crate::telemetry::ProxyMetrics>,
}

impl BackendAttemptObserver {
    pub fn new(
        health: Arc<PassiveBackendHealth>,
        #[cfg(feature = "telemetry")] metrics: Arc<crate::telemetry::ProxyMetrics>,
    ) -> Self {
        Self {
            health,
            #[cfg(feature = "telemetry")]
            metrics,
        }
    }
}

impl ConnectAttemptObserver for BackendAttemptObserver {
    fn on_attempt(&self, attempt: &ConnectAttempt<'_>) {
        self.health.on_attempt(attempt);

        #[cfg(feature = "telemetry")]
        self.metrics.record_backend_connect(
            attempt.elapsed.as_secs_f64(),
            attempt.server_id,
            &attempt.address.to_string(),
            attempt.succeeded(),
        );
    }
}

#[cfg(feature = "telemetry")]
pub struct HealthTransitionMetrics(pub Arc<crate::telemetry::ProxyMetrics>);

#[cfg(feature = "telemetry")]
impl HealthTransitionListener for HealthTransitionMetrics {
    fn on_transition(&self, address: &infrarust_config::ServerAddress, to: BackendState) {
        let to = match to {
            BackendState::Healthy => "healthy",
            BackendState::Probing => "probing",
            BackendState::Unhealthy => "unhealthy",
        };
        self.0
            .record_backend_health_transition(&address.to_string(), to);
    }
}
