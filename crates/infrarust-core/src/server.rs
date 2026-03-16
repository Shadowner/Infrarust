use std::sync::Arc;

use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use infrarust_config::{ProxyConfig, ProxyMode};
use infrarust_protocol::io::PacketEncoder;
use infrarust_protocol::packets::login::CLoginDisconnect;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use infrarust_protocol::{Packet, build_default_registry};
use infrarust_transport::{BackendConnector, Listener, ListenerConfig};
use tracing::Instrument;

use infrarust_api::events::proxy::ServerStateChangeEvent;
use infrarust_api::types::ServerId;
use infrarust_server_manager::ServerManagerService;

use crate::event_bus::EventBusImpl;
use crate::event_bus::conversion::convert_server_state;

use crate::auth::mojang::MojangAuth;
use crate::ban::file_storage::FileBanStorage;
use crate::ban::manager::BanManager;
use crate::ban::storage::BanStorage;
use crate::error::CoreError;
use crate::handler::client_only::ClientOnlyHandler;
use crate::handler::legacy::LegacyHandler;
use crate::handler::offline::OfflineHandler;
use crate::handler::passthrough::PassthroughHandler;
use crate::middleware::ban_check::BanCheckMiddleware;
use crate::middleware::ban_ip_check::BanIpCheckMiddleware;
use crate::middleware::domain_router::DomainRouterMiddleware;
use crate::middleware::handshake_parser::HandshakeParserMiddleware;
use crate::middleware::ip_filter::IpFilterMiddleware;
use crate::middleware::login_start_parser::LoginStartParserMiddleware;
use crate::middleware::rate_limiter::RateLimiterMiddleware;
use crate::middleware::server_manager::ServerManagerMiddleware;
use crate::middleware::telemetry::{ConnectionSpan, TelemetryMiddleware};
use crate::pipeline::Pipeline;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::MiddlewareResult;
use crate::pipeline::types::{ConnectionIntent, HandshakeData, LegacyDetected, RoutingData};
use crate::provider::file::FileProvider;
use crate::provider::registry::ProviderRegistry;
use crate::registry::ConnectionRegistry;
use crate::routing::DomainRouter;
use crate::status::{FaviconCache, StatusCache, StatusHandler, StatusRelayClient};

/// The main proxy server orchestrator.
///
/// Wires together the listener, pipelines, handlers, and config hot-reload.
pub struct ProxyServer {
    config: ProxyConfig,
    common_pipeline: Pipeline,
    login_pipeline: Pipeline,
    status_handler: StatusHandler,
    legacy_handler: LegacyHandler,
    passthrough_handler: PassthroughHandler,
    offline_handler: OfflineHandler,
    client_only_handler: ClientOnlyHandler,
    registry: Arc<ConnectionRegistry>,
    ban_manager: Arc<BanManager>,
    server_manager: Option<Arc<ServerManagerService>>,
    event_bus: Arc<EventBusImpl>,
    domain_router: Arc<DomainRouter>,
    packet_registry: Arc<PacketRegistry>,
    shutdown: CancellationToken,
}

impl ProxyServer {
    /// Builds the proxy server from config, loading server configs from disk.
    pub async fn new(config: ProxyConfig, shutdown: CancellationToken) -> Result<Self, CoreError> {
        // Create domain router (initially empty — providers populate it)
        let domain_router = Arc::new(DomainRouter::new());

        // Build packet registry
        let packet_registry = Arc::new(build_default_registry());

        // Create the event bus
        let event_bus = Arc::new(EventBusImpl::new());

        let backend_connector = Arc::new(BackendConnector::new(
            config.connect_timeout,
            config.keepalive.clone(),
        ));
        let registry = Arc::new(ConnectionRegistry::new());

        // Build status subsystem
        let status_cache = Arc::new(StatusCache::new(config.status_cache.ttl));
        let favicon_cache =
            Arc::new(FaviconCache::load_from_configs(&[], config.default_motd.as_ref()).await?);

        // --- Provider Registry: load initial configs ---
        let mut provider_registry = ProviderRegistry::new(
            Arc::clone(&domain_router),
            Arc::clone(&event_bus),
            Arc::clone(&status_cache),
            Arc::clone(&favicon_cache),
            shutdown.clone(),
        );

        // File provider (always enabled)
        provider_registry.add_provider(Box::new(FileProvider::new(config.servers_dir.clone())));

        // Docker provider (feature-gated)
        #[cfg(feature = "docker")]
        if let Some(ref docker_config) = config.docker {
            match crate::provider::docker::DockerProvider::new(docker_config) {
                Ok(docker_provider) => {
                    provider_registry.add_provider(Box::new(docker_provider));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to initialize docker provider, continuing without");
                }
            }
        }

        #[cfg(not(feature = "docker"))]
        if config.docker.is_some() {
            tracing::warn!(
                "docker configuration found but docker feature is not enabled, ignoring"
            );
        }

        // Start all providers (loads initial configs + starts watchers)
        let _provider_handle = provider_registry.start().await?;

        // Build server manager from configs that have a server_manager
        let managed_configs: Vec<(String, infrarust_config::ServerManagerConfig)> = domain_router
            .list_all()
            .iter()
            .filter_map(|(_pid, c)| {
                c.server_manager
                    .as_ref()
                    .map(|sm| (c.effective_id(), sm.clone()))
            })
            .collect();

        let server_manager = if managed_configs.is_empty() {
            None
        } else {
            let http_client = reqwest::Client::new();
            let service = ServerManagerService::new(&managed_configs, http_client);

            // Wire state change callback to fire ServerStateChangeEvent
            let bus = Arc::clone(&event_bus);
            service.set_on_state_change(Arc::new(move |server_id, old, new| {
                let api_old = convert_server_state(old);
                let api_new = convert_server_state(new);
                bus.fire_and_forget_arc(ServerStateChangeEvent {
                    server: ServerId::new(server_id),
                    old_state: api_old,
                    new_state: api_new,
                });
            }));

            tracing::info!(count = managed_configs.len(), "server manager initialized");
            Some(Arc::new(service))
        };

        // Load favicons from initial configs
        let favicon_configs: Vec<(String, Arc<infrarust_config::ServerConfig>)> = domain_router
            .list_all()
            .into_iter()
            .map(|(_pid, cfg)| (cfg.effective_id(), cfg))
            .collect();
        if let Err(e) = favicon_cache
            .reload(&favicon_configs, config.default_motd.as_ref())
            .await
        {
            tracing::warn!(error = %e, "failed to load initial favicons");
        }

        let relay_client = StatusRelayClient::new(
            Arc::clone(&backend_connector),
            Arc::clone(&packet_registry),
            std::time::Duration::from_secs(5),
        );

        let status_handler = StatusHandler::new(
            relay_client,
            Arc::clone(&status_cache),
            Arc::clone(&favicon_cache),
            server_manager.as_ref().map(Arc::clone),
            Arc::clone(&packet_registry),
            config.default_motd.clone(),
            Arc::clone(&event_bus),
        );

        let legacy_handler = LegacyHandler::new(
            Arc::clone(&domain_router),
            config.default_motd.clone(),
            server_manager.as_ref().map(Arc::clone),
            Arc::clone(&registry),
        );

        // Ban system
        let ban_storage = Arc::new(FileBanStorage::new(config.ban.file.clone()));
        ban_storage.load().await?;
        let ban_manager = Arc::new(BanManager::new(ban_storage, Arc::clone(&registry)));

        // Build common pipeline: IpFilter → BanIpCheck → HandshakeParser → RateLimiter → DomainRouter
        let mut common_pipeline = Pipeline::new();
        common_pipeline.add(Box::new(IpFilterMiddleware::new(None))); // Global filter from proxy config — Phase 2
        common_pipeline.add(Box::new(BanIpCheckMiddleware::new(Arc::clone(
            &ban_manager,
        ))));
        common_pipeline.add(Box::new(HandshakeParserMiddleware::new()));
        common_pipeline.add(Box::new(RateLimiterMiddleware::new(&config.rate_limit)));
        common_pipeline.add(Box::new(DomainRouterMiddleware::new(Arc::clone(
            &domain_router,
        ))));

        // Build login pipeline: LoginStartParser → BanCheck → Telemetry → ServerManager
        let mut login_pipeline = Pipeline::new();
        login_pipeline.add(Box::new(LoginStartParserMiddleware::new()));
        login_pipeline.add(Box::new(BanCheckMiddleware::new(Arc::clone(&ban_manager))));
        login_pipeline.add(Box::new(TelemetryMiddleware));
        if let Some(ref sm) = server_manager {
            login_pipeline.add(Box::new(ServerManagerMiddleware::new(Arc::clone(sm))));
        }

        // Build ProxyMetrics (telemetry feature only)
        #[cfg(feature = "telemetry")]
        let proxy_metrics = Arc::new(crate::telemetry::ProxyMetrics::new());

        let passthrough_handler = PassthroughHandler::new(
            Arc::clone(&backend_connector),
            Arc::clone(&packet_registry),
            Arc::clone(&registry),
        );
        #[cfg(feature = "telemetry")]
        let passthrough_handler = passthrough_handler.with_metrics(Arc::clone(&proxy_metrics));

        let offline_handler = OfflineHandler::new(
            Arc::clone(&backend_connector),
            Arc::clone(&packet_registry),
            Arc::clone(&registry),
        );
        #[cfg(feature = "telemetry")]
        let offline_handler = offline_handler.with_metrics(Arc::clone(&proxy_metrics));

        let auth = Arc::new(MojangAuth::new()?);
        let client_only_handler = ClientOnlyHandler::new(
            Arc::clone(&backend_connector),
            Arc::clone(&packet_registry),
            Arc::clone(&registry),
            auth,
        );
        #[cfg(feature = "telemetry")]
        let client_only_handler = client_only_handler.with_metrics(Arc::clone(&proxy_metrics));

        Ok(Self {
            config,
            common_pipeline,
            login_pipeline,
            status_handler,
            legacy_handler,
            passthrough_handler,
            offline_handler,
            client_only_handler,
            registry,
            ban_manager,
            server_manager,
            event_bus,
            domain_router,
            packet_registry,
            shutdown,
        })
    }

    /// Runs the proxy server, accepting connections until shutdown.
    pub async fn run(&self) -> Result<(), CoreError> {
        // Bind listener
        let listener_config = ListenerConfig {
            bind: self.config.bind,
            max_connections: self.config.max_connections,
            keepalive: self.config.keepalive.clone(),
            so_reuseport: self.config.so_reuseport,
            receive_proxy_protocol: self.config.receive_proxy_protocol,
        };

        let listener = Listener::bind(listener_config, self.shutdown.clone()).await?;

        tracing::info!(bind = %self.config.bind, "proxy server listening");

        // Start server manager health check and monitoring
        if let Some(ref sm) = self.server_manager {
            sm.initial_health_check().await;
            let player_counter: Arc<dyn infrarust_server_manager::PlayerCounter> =
                Arc::clone(&self.registry) as _;
            let _monitoring_handles = sm.start_monitoring(player_counter, self.shutdown.clone());
            tracing::info!("server manager monitoring started");
        }

        // Start ban purge task
        let _purge_handle = self
            .ban_manager
            .start_purge_task(self.config.ban.purge_interval, self.shutdown.clone());

        // Config hot-reload is handled by the ProviderRegistry (started in new())

        // Accept loop
        loop {
            let accepted = tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok(conn) => conn,
                        Err(e) => {
                            tracing::warn!(error = %e, "accept error");
                            continue;
                        }
                    }
                }
                () = self.shutdown.cancelled() => {
                    tracing::info!("proxy server shutting down");
                    break;
                }
            };

            let shutdown = self.shutdown.clone();
            let peer = accepted.connection.peer_addr();
            tracing::debug!(peer = %peer, "new connection");

            if let Err(e) = self.handle_connection(accepted, shutdown).await {
                tracing::warn!(peer = %peer, error = %e, "connection error");
            }
        }

        Ok(())
    }

    /// Processes a single connection through the pipeline.
    async fn handle_connection(
        &self,
        accepted: infrarust_transport::AcceptedConnection,
        shutdown: CancellationToken,
    ) -> Result<(), CoreError> {
        let mut ctx = ConnectionContext::from_accepted(accepted);

        // Execute common pipeline
        match self.common_pipeline.execute(&mut ctx).await? {
            MiddlewareResult::Continue => {}
            MiddlewareResult::ShortCircuit => {
                // Check if legacy was detected
                if ctx.extensions.contains::<LegacyDetected>() {
                    return self.legacy_handler.handle(&mut ctx).await;
                }
                return Ok(());
            }
            MiddlewareResult::Reject(msg) => {
                self.send_kick(&mut ctx, &msg).await.ok();
                return Ok(());
            }
        }

        // Branch on intent
        let intent = ctx
            .require_extension::<HandshakeData>("HandshakeData")?
            .intent;

        match intent {
            ConnectionIntent::Status => {
                self.status_handler.handle(&mut ctx, &self.registry).await?;
            }
            ConnectionIntent::Login => {
                // Execute login pipeline
                match self.login_pipeline.execute(&mut ctx).await? {
                    MiddlewareResult::Continue => {}
                    MiddlewareResult::ShortCircuit => return Ok(()),
                    MiddlewareResult::Reject(msg) => {
                        self.send_kick(&mut ctx, &msg).await.ok();
                        return Ok(());
                    }
                }

                // Route by proxy mode
                let proxy_mode = ctx
                    .require_extension::<RoutingData>("RoutingData")?
                    .server_config
                    .proxy_mode;

                // Extract the connection span (created by TelemetryMiddleware)
                let span = ctx
                    .extensions
                    .remove::<ConnectionSpan>()
                    .map(|cs| cs.0)
                    .unwrap_or_else(tracing::Span::none);

                match proxy_mode {
                    ProxyMode::Passthrough | ProxyMode::ZeroCopy | ProxyMode::ServerOnly => {
                        self.passthrough_handler
                            .handle(ctx, shutdown.child_token())
                            .instrument(span)
                            .await?;
                    }
                    ProxyMode::Offline => {
                        self.offline_handler
                            .handle(ctx, shutdown.child_token())
                            .instrument(span)
                            .await?;
                    }
                    ProxyMode::ClientOnly => {
                        self.client_only_handler
                            .handle(ctx, shutdown.child_token())
                            .instrument(span)
                            .await?;
                    }
                    ProxyMode::Full => {
                        tracing::warn!(
                            "Full mode not yet implemented, falling back to Passthrough"
                        );
                        self.passthrough_handler
                            .handle(ctx, shutdown.child_token())
                            .instrument(span)
                            .await?;
                    }
                    _ => {
                        self.passthrough_handler
                            .handle(ctx, shutdown.child_token())
                            .instrument(span)
                            .await?;
                    }
                }
            }
            ConnectionIntent::Transfer => {
                tracing::debug!("transfer intent — not supported in Phase 1");
            }
        }

        Ok(())
    }

    /// Sends a disconnect/kick packet to the client.
    async fn send_kick(&self, ctx: &mut ConnectionContext, reason: &str) -> Result<(), CoreError> {
        let json_reason = serde_json::json!({"text": reason}).to_string();

        let packet = CLoginDisconnect {
            reason: json_reason,
        };

        let version = ctx.extensions.get::<HandshakeData>().map_or(
            ProtocolVersion(infrarust_protocol::CURRENT_MC_PROTOCOL),
            |h| h.protocol_version,
        );

        let packet_id = self
            .packet_registry
            .get_packet_id::<CLoginDisconnect>(
                ConnectionState::Login,
                Direction::Clientbound,
                version,
            )
            .unwrap_or(0x00);

        let mut payload = Vec::new();
        packet.encode(&mut payload, version)?;

        let mut encoder = PacketEncoder::new();
        encoder.append_raw(packet_id, &payload)?;
        let bytes = encoder.take();

        ctx.stream_mut().write_all(&bytes).await?;
        ctx.stream_mut().flush().await?;

        Ok(())
    }

    /// Returns a reference to the connection registry.
    pub fn registry(&self) -> &ConnectionRegistry {
        &self.registry
    }

    /// Returns a reference to the ban manager.
    pub fn ban_manager(&self) -> &Arc<BanManager> {
        &self.ban_manager
    }

    /// Returns the event bus.
    pub fn event_bus(&self) -> &Arc<EventBusImpl> {
        &self.event_bus
    }

    /// Returns the domain router.
    pub fn domain_router(&self) -> &Arc<DomainRouter> {
        &self.domain_router
    }

    /// Returns the shutdown token.
    pub const fn shutdown(&self) -> &CancellationToken {
        &self.shutdown
    }
}
