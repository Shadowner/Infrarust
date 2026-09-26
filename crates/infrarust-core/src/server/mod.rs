mod accept;
mod wiring;

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use infrarust_config::UnknownDomainBehavior;
use infrarust_transport::{Listener, ListenerConfig};

use crate::ban::manager::BanManager;
use crate::error::CoreError;
use crate::event_bus::EventBusImpl;
use crate::handler::InterceptedHandler;
use crate::handler::legacy::LegacyHandler;
use crate::handler::passthrough::PassthroughHandler;
use crate::pipeline::Pipeline;
use crate::routing::DomainRouter;
use crate::services::ProxyServices;
use crate::session::connection_registry::ConnectionRegistry;
use crate::status::StatusHandler;

pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ProxyServer {
    common_pipeline: Pipeline,
    login_pipeline: Pipeline,
    status_handler: StatusHandler,
    legacy_handler: LegacyHandler,
    passthrough_handler: PassthroughHandler,
    offline_handler: InterceptedHandler,
    client_only_handler: InterceptedHandler,
    services: ProxyServices,
    backend_health: Arc<crate::loadbalancer::PassiveBackendHealth>,
    unknown_domain_behavior: UnknownDomainBehavior,
    shutdown: CancellationToken,
    sessions: CancellationToken,
    background: CancellationToken,
    connections: TaskTracker,
}

impl ProxyServer {
    pub async fn run(self: Arc<Self>) -> Result<(), CoreError> {
        let listener = match self.bind().await {
            Ok(listener) => listener,
            Err(e) => {
                self.stop_background_tasks();
                return Err(e);
            }
        };
        let result = Arc::clone(&self).serve(listener).await;
        self.close_sessions();
        self.drain_connections(DEFAULT_DRAIN_TIMEOUT).await;
        self.stop_background_tasks();
        result
    }

    pub async fn bind(&self) -> Result<Listener, CoreError> {
        let config = &self.services.config;
        let listener_config = ListenerConfig {
            bind: config.bind,
            max_connections: config.max_connections,
            keepalive: config.keepalive.clone(),
            so_reuseport: config.so_reuseport,
            receive_proxy_protocol: config.receive_proxy_protocol,
        };

        let listener = Listener::bind(listener_config, self.shutdown.clone()).await?;

        tracing::info!(bind = %listener.local_addr()?, "proxy server listening");
        Ok(listener)
    }

    pub fn close_sessions(&self) {
        self.sessions.cancel();
    }

    pub async fn drain_connections(&self, timeout: Duration) {
        self.connections.close();
        let remaining = self.connections.len();
        if remaining == 0 {
            return;
        }
        tracing::info!(remaining, "waiting for active connections to drain");
        match tokio::time::timeout(timeout, self.connections.wait()).await {
            Ok(()) => tracing::info!("all connections drained"),
            Err(_) => tracing::warn!(
                remaining = self.connections.len(),
                "drain timeout, forcing shutdown"
            ),
        }
    }

    pub fn active_connections(&self) -> usize {
        self.connections.len()
    }

    pub fn stop_background_tasks(&self) {
        self.background.cancel();
    }

    pub const fn background_token(&self) -> &CancellationToken {
        &self.background
    }

    pub const fn services(&self) -> &ProxyServices {
        &self.services
    }

    pub fn registry(&self) -> &ConnectionRegistry {
        &self.services.connection_registry
    }

    pub fn ban_manager(&self) -> &Arc<BanManager> {
        &self.services.ban_manager
    }

    pub fn event_bus(&self) -> &Arc<EventBusImpl> {
        &self.services.event_bus
    }

    pub fn domain_router(&self) -> &Arc<DomainRouter> {
        &self.services.domain_router
    }

    pub const fn shutdown(&self) -> &CancellationToken {
        &self.shutdown
    }
}

impl Drop for ProxyServer {
    fn drop(&mut self) {
        self.sessions.cancel();
        self.background.cancel();
    }
}
