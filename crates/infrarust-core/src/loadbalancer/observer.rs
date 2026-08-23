//! Fan-out of backend connect outcomes to health, telemetry and the event bus.

use std::sync::Arc;

use infrarust_api::events::proxy::BackendHealthEvent;
use infrarust_api::types::{ServerAddress as ApiServerAddress, ServerId};
use infrarust_transport::{ConnectAttempt, ConnectAttemptObserver};

use super::{BackendState, HealthTransitionListener, PassiveBackendHealth};
use crate::event_bus::EventBusImpl;
use crate::routing::DomainRouter;

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
        self.0
            .record_backend_health_transition(&address.to_string(), to.to_api().as_str());
    }
}

/// Publishes health transitions on the event bus, resolved back to the
/// servers that list the address.
pub struct HealthTransitionEvents {
    router: Arc<DomainRouter>,
    event_bus: Arc<EventBusImpl>,
}

impl HealthTransitionEvents {
    pub fn new(router: Arc<DomainRouter>, event_bus: Arc<EventBusImpl>) -> Self {
        Self { router, event_bus }
    }
}

impl HealthTransitionListener for HealthTransitionEvents {
    fn on_transition(&self, address: &infrarust_config::ServerAddress, to: BackendState) {
        let servers: Vec<ServerId> = self
            .router
            .list_all()
            .into_iter()
            .filter(|(_, config)| config.addresses.iter().any(|wa| wa.address == *address))
            .map(|(_, config)| ServerId::new(config.effective_id()))
            .collect();

        self.event_bus.fire_and_forget_arc(BackendHealthEvent {
            address: ApiServerAddress {
                host: address.host.clone(),
                port: address.port,
            },
            servers,
            state: to.to_api(),
        });
    }
}
