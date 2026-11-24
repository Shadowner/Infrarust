mod accept;
mod health_observer;
mod health_prober;
mod proxy_services;
mod status_refresh;
mod wiring;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::JoinHandle;
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
use crate::handoff::HandoffRelay;
use crate::hot_restart::RelayRegistry;
use crate::middleware::rate_limiter::RateLimiterMiddleware;
use crate::pipeline::Pipeline;
use crate::player::ShutdownKind;
use crate::registry::ConnectionRegistry;
use crate::routing::DomainRouter;
use crate::status::StatusHandler;

pub use proxy_services::ProxyServices;

pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
const PIPELINE_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub enum ServeExit {
    Stopped,
    Restart(Box<Listener>),
}

pub struct ProxyServer {
    common_pipeline: Pipeline,
    login_pipeline: Pipeline,
    rate_limiter: RateLimiterMiddleware,
    status_handler: StatusHandler,
    legacy_handler: LegacyHandler,
    passthrough_handler: PassthroughHandler,
    offline_handler: InterceptedHandler,
    client_only_handler: InterceptedHandler,
    handoff: HandoffRelay,
    services: ProxyServices,
    backend_health: Arc<crate::loadbalancer::PassiveBackendHealth>,
    unknown_domain_behavior: UnknownDomainBehavior,
    shutdown: CancellationToken,
    background: CancellationToken,
    listener_request: Notify,
    connections: TaskTracker,
    #[cfg(feature = "telemetry")]
    metrics: Arc<crate::telemetry::ProxyMetrics>,
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
        self.close_sessions(ShutdownKind::Stop);
        self.drain_connections(DEFAULT_DRAIN_TIMEOUT).await;
        self.stop_background_tasks();
        result.map(|_| ())
    }

    pub async fn bind(&self) -> Result<Listener, CoreError> {
        self.bind_or_adopt(None).await
    }

    pub(crate) async fn bind_or_adopt(
        &self,
        inherited: Option<std::net::TcpListener>,
    ) -> Result<Listener, CoreError> {
        let config = &self.services.config;
        let listener_config = ListenerConfig {
            bind: config.bind,
            max_connections: config.max_connections,
            max_connections_per_ip: config.max_connections_per_ip,
            keepalive: config.keepalive.clone(),
            so_reuseport: config.so_reuseport,
            receive_proxy_protocol: config.receive_proxy_protocol,
            trusted_proxies: config.trusted_proxies.clone(),
        };

        let listener = match inherited {
            Some(inherited) => Listener::adopt(inherited, listener_config, self.shutdown.clone())?,
            None => Listener::bind(listener_config, self.shutdown.clone())?,
        };

        tracing::info!(bind = %listener.local_addr(), "proxy server listening");
        Ok(listener)
    }

    pub fn close_sessions(&self, kind: ShutdownKind) {
        let relays = &self.services.relays;
        relays.set_notice(kind);
        match kind {
            ShutdownKind::Stop => relays.sessions().cancel(),
            ShutdownKind::Restart => relays.close_pipeline(),
        }
    }

    pub fn request_listener(&self) {
        self.listener_request.notify_one();
    }

    pub async fn drain_pipeline(&self, timeout: Duration) -> usize {
        let mut ticks = tokio::time::interval(PIPELINE_POLL_INTERVAL);
        let drained = tokio::time::timeout(timeout, async {
            while self.pipeline_connections() > 0 {
                ticks.tick().await;
            }
        })
        .await;
        let left = self.pipeline_connections();
        if drained.is_err() {
            tracing::warn!(
                left,
                "connections outside the forwarders were still open when the pipeline drain timed out"
            );
        }
        left
    }

    pub fn pipeline_connections(&self) -> usize {
        self.connections
            .len()
            .saturating_sub(self.services.relays.active())
    }

    pub fn track_session<F>(&self, session: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.connections.spawn(session)
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

    pub async fn settle_connections(&self, timeout: Duration) -> usize {
        self.connections.close();
        let _ = tokio::time::timeout(timeout, self.connections.wait()).await;
        self.connections.len()
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

    pub const fn handoff(&self) -> &HandoffRelay {
        &self.handoff
    }

    pub fn relays(&self) -> &Arc<RelayRegistry> {
        &self.services.relays
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

    #[cfg(feature = "telemetry")]
    pub(crate) const fn metrics(&self) -> &Arc<crate::telemetry::ProxyMetrics> {
        &self.metrics
    }
}

impl Drop for ProxyServer {
    fn drop(&mut self) {
        self.services.relays.sessions().cancel();
        self.background.cancel();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use infrarust_config::ProxyConfig;
    use tokio::net::TcpStream;

    use super::*;

    const T: Duration = Duration::from_secs(5);

    async fn server(dir: &std::path::Path) -> Arc<ProxyServer> {
        let servers_dir = dir.join("servers");
        std::fs::create_dir(&servers_dir).unwrap();
        let mut config: ProxyConfig = toml::from_str("bind = \"127.0.0.1:0\"\n").unwrap();
        config.servers_dir = servers_dir;
        config.plugins_dir = dir.join("plugins");
        config.ban.file = dir.join("bans.json");
        let server = ProxyServer::new(config, dir.join("infrarust.toml"), CancellationToken::new())
            .await
            .unwrap();
        Arc::new(server)
    }

    #[tokio::test]
    async fn a_listener_request_hands_back_a_listener_that_keeps_queueing() {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path()).await;
        let listener = server.bind().await.unwrap();
        let addr = listener.local_addr();
        let serving = tokio::spawn(Arc::clone(&server).serve(listener));

        server.request_listener();
        let exit = tokio::time::timeout(T, serving)
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        let ServeExit::Restart(listener) = exit else {
            panic!("a listener request must hand the listener back, got {exit:?}");
        };
        assert_eq!(listener.local_addr(), addr);
        let client = TcpStream::connect(addr).await.unwrap();
        assert_eq!(server.active_connections(), 0);
        let pending = tokio::time::timeout(T, listener.accept())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pending.peer_addr(), client.local_addr().unwrap());
    }

    #[tokio::test]
    async fn the_shutdown_token_stops_the_accept_loop() {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path()).await;
        let listener = server.bind().await.unwrap();
        let serving = tokio::spawn(Arc::clone(&server).serve(listener));

        server.shutdown().cancel();

        let exit = tokio::time::timeout(T, serving)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(exit, ServeExit::Stopped), "{exit:?}");
    }

    #[tokio::test]
    async fn closing_for_a_restart_cuts_the_pipeline_and_keeps_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path()).await;
        let relays = Arc::clone(server.relays());
        let (forwarding, link) = relays.link_login();
        assert!(link.detach());
        let (logging_in, _link) = relays.link_login();

        server.close_sessions(ShutdownKind::Restart);

        assert_eq!(relays.notice(), ShutdownKind::Restart);
        assert!(relays.pipeline().is_cancelled());
        assert!(logging_in.is_cancelled());
        assert!(!forwarding.is_cancelled());

        server.close_sessions(ShutdownKind::Stop);

        assert_eq!(relays.notice(), ShutdownKind::Stop);
        assert!(forwarding.is_cancelled());
    }

    #[tokio::test]
    async fn the_pipeline_drains_down_to_the_forwarders() {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path()).await;
        let relays = Arc::clone(server.relays());
        let ticket = relays.ticket();
        let forwarder = server.track_session(std::future::pending::<()>());
        let release = CancellationToken::new();
        let gate = release.clone();
        server.track_session(async move { gate.cancelled().await });

        assert_eq!(server.pipeline_connections(), 1);
        assert_eq!(server.drain_pipeline(Duration::from_millis(30)).await, 1);

        release.cancel();
        assert_eq!(server.drain_pipeline(T).await, 0);
        assert_eq!(server.active_connections(), 1);
        forwarder.abort();
        drop(ticket);
    }
}
