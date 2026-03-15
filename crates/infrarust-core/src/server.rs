use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use infrarust_config::{DomainIndex, ProxyConfig, ProxyMode, ServerConfig};
use infrarust_protocol::io::PacketEncoder;
use infrarust_protocol::packets::login::CLoginDisconnect;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use infrarust_protocol::{Packet, build_default_registry};
use infrarust_transport::{BackendConnector, Listener, ListenerConfig};

use infrarust_server_manager::ServerManagerService;

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
use crate::pipeline::Pipeline;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::MiddlewareResult;
use crate::pipeline::types::{ConnectionIntent, HandshakeData, LegacyDetected, RoutingData};
use crate::provider::file::FileProvider;
use crate::registry::ConnectionRegistry;
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
    domain_index: Arc<ArcSwap<DomainIndex>>,
    configs: Arc<ArcSwap<HashMap<String, Arc<ServerConfig>>>>,
    packet_registry: Arc<PacketRegistry>,
    shutdown: CancellationToken,
}

impl ProxyServer {
    /// Builds the proxy server from config, loading server configs from disk.
    pub async fn new(config: ProxyConfig, shutdown: CancellationToken) -> Result<Self, CoreError> {
        // Load initial server configs
        let provider = FileProvider::new(config.servers_dir.clone());
        let server_configs = provider.load_configs()?;

        // Build domain index and config map
        let domain_index = Arc::new(ArcSwap::from_pointee(DomainIndex::build(&server_configs)));
        let config_map: HashMap<String, Arc<ServerConfig>> = server_configs
            .into_iter()
            .map(|c| (c.effective_id(), Arc::new(c)))
            .collect();

        // Build server manager from configs that have a server_manager
        let managed_configs: Vec<(String, infrarust_config::ServerManagerConfig)> = config_map
            .values()
            .filter_map(|c| {
                c.server_manager
                    .as_ref()
                    .map(|sm| (c.effective_id(), sm.clone()))
            })
            .collect();

        let configs = Arc::new(ArcSwap::from_pointee(config_map));

        // Build packet registry
        let packet_registry = Arc::new(build_default_registry());

        let server_manager = if managed_configs.is_empty() {
            None
        } else {
            let http_client = reqwest::Client::new();
            let service = ServerManagerService::new(&managed_configs, http_client);
            tracing::info!(count = managed_configs.len(), "server manager initialized");
            Some(Arc::new(service))
        };

        let backend_connector = Arc::new(BackendConnector::new(
            config.connect_timeout,
            config.keepalive.clone(),
        ));
        let registry = Arc::new(ConnectionRegistry::new());

        // Build status subsystem
        let status_cache = Arc::new(StatusCache::new(config.status_cache.ttl));

        let favicon_configs: Vec<(String, Arc<ServerConfig>)> = configs
            .load()
            .iter()
            .map(|(id, cfg)| (id.clone(), Arc::clone(cfg)))
            .collect();
        let favicon_cache = Arc::new(
            FaviconCache::load_from_configs(&favicon_configs, config.default_motd.as_ref()).await?,
        );

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
        );

        let legacy_handler = LegacyHandler::new(
            Arc::clone(&domain_index),
            Arc::clone(&configs),
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
        common_pipeline.add(Box::new(DomainRouterMiddleware::new(
            Arc::clone(&domain_index),
            Arc::clone(&configs),
        )));

        // Build login pipeline: LoginStartParser → BanCheck → ServerManager
        let mut login_pipeline = Pipeline::new();
        login_pipeline.add(Box::new(LoginStartParserMiddleware::new()));
        login_pipeline.add(Box::new(BanCheckMiddleware::new(Arc::clone(&ban_manager))));
        if let Some(ref sm) = server_manager {
            login_pipeline.add(Box::new(ServerManagerMiddleware::new(Arc::clone(sm))));
        }

        let passthrough_handler =
            PassthroughHandler::new(Arc::clone(&backend_connector), Arc::clone(&registry));

        let offline_handler = OfflineHandler::new(
            Arc::clone(&backend_connector),
            Arc::clone(&packet_registry),
            Arc::clone(&registry),
        );

        let auth = Arc::new(MojangAuth::new()?);
        let client_only_handler = ClientOnlyHandler::new(
            Arc::clone(&backend_connector),
            Arc::clone(&packet_registry),
            Arc::clone(&registry),
            auth,
        );

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
            domain_index,
            configs,
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

        // Start config hot-reload
        let provider = FileProvider::new(self.config.servers_dir.clone());
        if let Ok((rx, _watcher)) = provider.watch() {
            let domain_index = Arc::clone(&self.domain_index);
            let configs = Arc::clone(&self.configs);
            let status_cache = Arc::clone(self.status_handler.cache());
            let favicon_cache = Arc::clone(self.status_handler.favicon_cache());
            let shutdown = self.shutdown.clone();
            tokio::spawn(crate::reload::run_config_watcher(
                rx,
                domain_index,
                configs,
                status_cache,
                favicon_cache,
                shutdown,
            ));
        }

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

                match proxy_mode {
                    ProxyMode::Passthrough | ProxyMode::ZeroCopy | ProxyMode::ServerOnly => {
                        self.passthrough_handler
                            .handle(ctx, shutdown.child_token())
                            .await?;
                    }
                    ProxyMode::Offline => {
                        self.offline_handler
                            .handle(ctx, shutdown.child_token())
                            .await?;
                    }
                    ProxyMode::ClientOnly => {
                        self.client_only_handler
                            .handle(ctx, shutdown.child_token())
                            .await?;
                    }
                    ProxyMode::Full => {
                        tracing::warn!(
                            "Full mode not yet implemented, falling back to Passthrough"
                        );
                        self.passthrough_handler
                            .handle(ctx, shutdown.child_token())
                            .await?;
                    }
                    _ => {
                        self.passthrough_handler
                            .handle(ctx, shutdown.child_token())
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

    /// Returns the shutdown token.
    pub const fn shutdown(&self) -> &CancellationToken {
        &self.shutdown
    }
}
