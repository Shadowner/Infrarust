use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use infrarust_config::{
    ForwardingMode as ConfigForwardingMode, ProxyConfig, ProxyMode, UnknownDomainBehavior,
};
use infrarust_protocol::build_default_registry;
use infrarust_protocol::version::ProtocolVersion;
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
use crate::handler::InterceptedHandler;
use crate::handler::legacy::LegacyHandler;
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
use crate::player::registry::PlayerRegistryImpl;
use crate::provider::file::FileProvider;
use crate::provider::registry::ProviderRegistry;
use crate::registry::ConnectionRegistry;
use crate::routing::DomainRouter;
use crate::services::ProxyServices;
use crate::services::command_manager::CommandManagerImpl;
use crate::status::{FaviconCache, StatusCache, StatusHandler, StatusRelayClient};

/// The main proxy server orchestrator.
///
/// Wires together the listener, pipelines, handlers, and config hot-reload.
pub struct ProxyServer {
    common_pipeline: Pipeline,
    login_pipeline: Pipeline,
    status_handler: StatusHandler,
    legacy_handler: LegacyHandler,
    passthrough_handler: PassthroughHandler,
    offline_handler: InterceptedHandler,
    client_only_handler: InterceptedHandler,
    services: ProxyServices,
    unknown_domain_behavior: UnknownDomainBehavior,
    shutdown: CancellationToken,
}

impl ProxyServer {
    /// Builds the proxy server from config, loading server configs from disk.
    ///
    /// # Errors
    /// Returns `CoreError` on favicon loading, provider initialization,
    /// or Mojang auth key generation failures.
    #[allow(clippy::too_many_lines)]
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
        let (_provider_handle, provider_event_sender) = provider_registry.start().await?;

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
            service.add_on_state_change(Arc::new(move |server_id, old, new| {
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
            Arc::clone(&backend_connector),
            shutdown.clone(),
        );

        // Ban system
        let ban_storage = Arc::new(FileBanStorage::new(config.ban.file.clone()));
        ban_storage.load().await?;
        let ban_manager = Arc::new(BanManager::new(ban_storage, Arc::clone(&registry)));

        // Build plugin services
        let player_registry = Arc::new(PlayerRegistryImpl::new(Arc::clone(&registry)));
        let command_manager = Arc::new(CommandManagerImpl::new());

        let codec_filter_registry =
            Arc::new(crate::filter::codec_registry::CodecFilterRegistryImpl::new());

        let limbo_handler_registry = Arc::new(crate::limbo::registry::LimboHandlerRegistry::new());

        let forwarding_mode = Arc::new(Self::resolve_forwarding_mode(&config));
        let forwarding_secret = Self::load_forwarding_secret(&config, &forwarding_mode);

        let permission_service =
            Arc::new(crate::permissions::PermissionService::new(&config.permissions).await);

        let services = ProxyServices {
            event_bus: Arc::clone(&event_bus),
            player_registry,
            command_manager,
            connection_registry: Arc::clone(&registry),
            packet_registry: Arc::clone(&packet_registry),
            server_manager: server_manager.clone(),
            ban_manager: Arc::clone(&ban_manager),
            config: Arc::new(config.clone()),
            domain_router: Arc::clone(&domain_router),
            codec_filter_registry: Arc::clone(&codec_filter_registry),
            transport_filter_chain: crate::filter::transport_chain::TransportFilterChain::empty(),
            limbo_handler_registry,
            registry_codec_cache: Arc::new(crate::limbo::registry_cache::RegistryCodecCache::new(
                Arc::new(crate::registry_data::embedded::EmbeddedRegistryDataProvider),
            )),
            provider_event_sender,
            forwarding_mode,
            forwarding_secret,
            permission_service,
        };

        // Build common pipeline: IpFilter → BanIpCheck → HandshakeParser → RateLimiter → DomainRouter
        let mut common_pipeline = Pipeline::new();
        common_pipeline.add(Box::new(IpFilterMiddleware::new(config.ip_filter.clone())));
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

        let passthrough_handler =
            PassthroughHandler::new(Arc::clone(&backend_connector), services.clone());
        #[cfg(feature = "telemetry")]
        let passthrough_handler = passthrough_handler.with_metrics(Arc::clone(&proxy_metrics));

        let auth = Arc::new(MojangAuth::new()?);

        let offline_handler = InterceptedHandler::offline(
            Arc::clone(&backend_connector),
            services.clone(),
            Some(Arc::clone(&auth)),
        );
        #[cfg(feature = "telemetry")]
        let offline_handler = offline_handler.with_metrics(Arc::clone(&proxy_metrics));

        let client_only_handler =
            InterceptedHandler::client_only(Arc::clone(&backend_connector), services.clone(), auth);
        #[cfg(feature = "telemetry")]
        let client_only_handler = client_only_handler.with_metrics(Arc::clone(&proxy_metrics));

        Ok(Self {
            common_pipeline,
            login_pipeline,
            status_handler,
            legacy_handler,
            passthrough_handler,
            offline_handler,
            client_only_handler,
            services,
            unknown_domain_behavior: config.unknown_domain_behavior,
            shutdown,
        })
    }

    /// Runs the proxy server, accepting connections until shutdown.
    ///
    /// # Errors
    /// Returns `CoreError` if the listener fails to bind.
    pub async fn run(self: Arc<Self>) -> Result<(), CoreError> {
        // Bind listener
        let config = &self.services.config;
        let listener_config = ListenerConfig {
            bind: config.bind,
            max_connections: config.max_connections,
            keepalive: config.keepalive.clone(),
            so_reuseport: config.so_reuseport,
            receive_proxy_protocol: config.receive_proxy_protocol,
        };

        let listener = Listener::bind(listener_config, self.shutdown.clone()).await?;

        tracing::info!(bind = %config.bind, "proxy server listening");

        // Start server manager health check and monitoring
        if let Some(ref sm) = self.services.server_manager {
            sm.initial_health_check().await;
            let player_counter: Arc<dyn infrarust_server_manager::PlayerCounter> =
                Arc::clone(&self.services.connection_registry) as _;
            let _monitoring_handles = sm.start_monitoring(player_counter, self.shutdown.clone());
            tracing::info!("server manager monitoring started");
        }

        // Start ban purge task
        let _purge_handle = self
            .services
            .ban_manager
            .start_purge_task(config.ban.purge_interval, self.shutdown.clone());

        // Config hot-reload is handled by the ProviderRegistry (started in new())

        // Accept loop
        loop {
            let accepted = tokio::select! {
                biased;
                () = self.shutdown.cancelled() => {
                    tracing::info!("proxy server shutting down");
                    break;
                }
                result = listener.accept() => {
                    match result {
                        Ok(conn) => conn,
                        Err(e) => {
                            tracing::warn!(error = %e, "accept error");
                            continue;
                        }
                    }
                }
            };

            let shutdown = self.shutdown.clone();
            let peer = accepted.connection.peer_addr();
            let local = accepted.connection.local_addr();
            tracing::debug!(peer = %peer, "new connection");

            // Transport filter: on_accept
            // Runs BEFORE the connection pipeline — real_ip is not yet available
            // (set by PROXY protocol middleware later) and connection_id is 0
            // (assigned per-session, not per-accept).
            if !self.services.transport_filter_chain.is_empty() {
                use infrarust_api::filter::{FilterVerdict, TransportContext};
                use infrarust_api::types::Extensions;

                let mut transport_ctx = TransportContext {
                    remote_addr: peer,
                    local_addr: local,
                    real_ip: None,
                    connection_time: std::time::Instant::now(),
                    bytes_received: 0,
                    bytes_sent: 0,
                    connection_id: 0,
                    extensions: Extensions::new(),
                };

                if matches!(
                    self.services
                        .transport_filter_chain
                        .on_accept(&mut transport_ctx)
                        .await,
                    FilterVerdict::Reject
                ) {
                    tracing::debug!(peer = %peer, "Connection rejected by transport filter");
                    drop(accepted);
                    continue;
                }
            }

            // TODO: on_client_data/on_server_data wrapping
            // These require wrapping the TCP stream to intercept raw bytes.
            // Will be implemented when a real use case demands it.

            let server = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(e) = server.handle_connection(accepted, shutdown).await {
                    tracing::warn!(peer = %peer, error = %e, "connection error");
                }
            });
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
                if self.unknown_domain_behavior == UnknownDomainBehavior::Drop {
                    tracing::debug!("dropping connection: {msg}");
                    return Ok(());
                }
                let is_status = ctx
                    .extensions
                    .get::<HandshakeData>()
                    .is_some_and(|h| h.intent == ConnectionIntent::Status);
                if !is_status {
                    self.send_kick(&mut ctx, &msg).await.ok();
                }
                return Ok(());
            }
        }

        // Branch on intent
        let intent = ctx
            .require_extension::<HandshakeData>("HandshakeData")?
            .intent;

        match intent {
            ConnectionIntent::Status => {
                self.status_handler
                    .handle(&mut ctx, &self.services.connection_registry)
                    .await?;
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
                    .map_or_else(tracing::Span::none, |cs| cs.0);

                match proxy_mode {
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
        let version = ctx.extensions.get::<HandshakeData>().map_or(
            ProtocolVersion(infrarust_protocol::CURRENT_MC_PROTOCOL),
            |h| h.protocol_version,
        );

        crate::handler::helpers::send_login_disconnect(
            ctx.stream_mut(),
            reason,
            version,
            &self.services.packet_registry,
        )
        .await
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

    /// Rebuilds the transport filter chain from a registry.
    ///
    /// Call this after plugins have been loaded and may have registered
    /// transport filters.
    pub fn rebuild_transport_filter_chain(
        &mut self,
        registry: &crate::filter::transport_registry::TransportFilterRegistryImpl,
    ) {
        self.services.transport_filter_chain = registry.build_chain();
    }

    pub const fn shutdown(&self) -> &CancellationToken {
        &self.shutdown
    }

    fn load_forwarding_secret(
        config: &ProxyConfig,
        mode: &crate::forwarding::ForwardingMode,
    ) -> Option<Arc<[u8]>> {
        // If the mode already has a secret, use it
        match mode {
            crate::forwarding::ForwardingMode::Velocity { secret } => {
                return Some(secret.as_slice().into());
            }
            crate::forwarding::ForwardingMode::BungeeGuard { token } => {
                return Some(token.as_bytes().into());
            }
            _ => {}
        }

        let secret_path = config
            .forwarding
            .as_ref()
            .map(|f| f.secret_file.clone())
            .unwrap_or_else(|| std::path::PathBuf::from("forwarding.secret"));

        if secret_path.exists() {
            match crate::forwarding::secret::load_or_generate_secret(&secret_path) {
                Ok(secret) => {
                    tracing::info!(
                        path = %secret_path.display(),
                        "loaded forwarding secret for Velocity auto-detection"
                    );
                    Some(secret.into())
                }
                Err(e) => {
                    tracing::debug!(
                        path = %secret_path.display(),
                        error = %e,
                        "could not load forwarding secret — Velocity auto-detection disabled"
                    );
                    None
                }
            }
        } else {
            None
        }
    }

    fn resolve_forwarding_mode(config: &ProxyConfig) -> crate::forwarding::ForwardingMode {
        let fwd_config = match &config.forwarding {
            Some(c) => c,
            None => return crate::forwarding::ForwardingMode::None,
        };

        match &fwd_config.mode {
            ConfigForwardingMode::None => crate::forwarding::ForwardingMode::None,
            ConfigForwardingMode::BungeeCord => {
                tracing::warn!(
                    "BungeeCord legacy forwarding is configured globally. \
                     This mode is fundamentally insecure — anyone reaching the backend \
                     directly can impersonate any player. Consider migrating to Velocity \
                     modern forwarding."
                );
                crate::forwarding::ForwardingMode::BungeeCord
            }
            ConfigForwardingMode::BungeeGuard => {
                match crate::forwarding::secret::load_or_generate_secret(&fwd_config.secret_file) {
                    Ok(secret) => {
                        let token = String::from_utf8_lossy(&secret).to_string();
                        crate::forwarding::ForwardingMode::BungeeGuard { token }
                    }
                    Err(e) => {
                        tracing::error!(
                            path = %fwd_config.secret_file.display(),
                            error = %e,
                            "failed to load BungeeGuard secret, falling back to no forwarding"
                        );
                        crate::forwarding::ForwardingMode::None
                    }
                }
            }
            ConfigForwardingMode::Velocity => {
                match crate::forwarding::secret::load_or_generate_secret(&fwd_config.secret_file) {
                    Ok(secret) => crate::forwarding::ForwardingMode::Velocity { secret },
                    Err(e) => {
                        tracing::error!(
                            path = %fwd_config.secret_file.display(),
                            error = %e,
                            "failed to load Velocity secret, falling back to no forwarding"
                        );
                        crate::forwarding::ForwardingMode::None
                    }
                }
            }
        }
    }
}
