//! Infrarust - A Minecraft proxy server implementation in Rust
//!
//! This crate provides a flexible and efficient proxy server for Minecraft,
//! supporting multiple backend servers, protocol versions, and various security features.
//! It's designed to proxy multiple domain names to different type of Minecraft servers

// Core modules
pub mod core;
pub use core::config::InfrarustConfig;
use core::config::provider::ConfigProvider;
use core::config::provider::file::FileProvider;
use core::config::service::ConfigurationService;
pub use core::error::RsaError;
use core::error::SendError;
use core::shared_component::SharedComponent;
use core::{actors::supervisor::ActorSupervisor, config::provider::docker};
use std::io;
use std::net::IpAddr;
use std::sync::Arc;

pub mod telemetry;

// Protocol modules
pub mod protocol;
use infrarust_ban_system::BanEntry;
use protocol::minecraft::java::handshake::ServerBoundHandshake;
pub use protocol::{
    types::{ProtocolRead, ProtocolWrite},
    version,
};
use security::filter::FilterError;
use tracing::{Instrument, Span, debug, debug_span, error, info, instrument, warn};

// Network and security modules
pub mod network;
pub mod security;
pub use network::{
    connection::{Connection, ServerConnection},
    proxy_protocol::{ProxyProtocolConfig, write_proxy_protocol_header},
};
pub mod proxy_modes;
use security::ban_system_adapter::BanSystemAdapter;
pub use security::{
    encryption::EncryptionState,
    filter::{Filter, FilterConfig, FilterRegistry, FilterType},
    rate_limiter::RateLimiter,
};

// Server implementation
pub mod cli;
pub mod server;
use cli::ShutdownController;
use server::ServerRequest;
use server::gateway::Gateway;
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::version::Version;
#[derive(Debug)]
pub struct Infrarust {
    shared: Arc<SharedComponent>,
    gateway: Arc<Gateway>,
}

impl Infrarust {
    pub fn new(
        config: InfrarustConfig,
        shutdown_controller: Arc<ShutdownController>,
    ) -> io::Result<Self> {
        let span = debug_span!("infrarust_init");
        let _enter = span.enter();
        let config = Arc::new(config);

        debug!("Initializing Infrarust server with config: {:?}", config);
        let config_service = Arc::new(ConfigurationService::new());

        let (gateway_sender, gateway_receiver) = tokio::sync::mpsc::channel(100);
        let (provider_sender, provider_receiver) = tokio::sync::mpsc::channel(100);

        // Initialize filter registry
        let filter_registry = Arc::new(FilterRegistry::new());
        if ActorSupervisor::initialize_global().is_err() {
            error!("Failed to initialize ActorSupervisor");
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Failed to initialize ActorSupervisor",
            ));
        }

        let supervisor = ActorSupervisor::global();

        let mut config_provider = ConfigProvider::new(
            config_service.clone(),
            provider_receiver,
            provider_sender.clone(),
        );

        let shared = Arc::new(SharedComponent::new(
            config,
            supervisor,
            config_service,
            filter_registry,
            shutdown_controller,
            gateway_sender,
            provider_sender,
        ));

        let server_gateway = Arc::new(Gateway::new(shared.clone()));
        if let Some(file_config) = shared.config().file_provider.clone() {
            let file_provider = FileProvider::new(
                file_config.proxies_path,
                file_config.file_type,
                file_config.watch,
                shared.provider_sender().clone(),
            );

            config_provider.register_provider(Box::new(file_provider));
        }

        if let Some(docker_config) = shared.config().docker_provider.clone() {
            let docker_provider = Box::new(docker::DockerProvider::new(
                docker_config,
                shared.provider_sender().clone(),
            ));
            config_provider.register_provider(docker_provider);
            info!("Docker provider registered");
        }

        let provider_span = Span::current();
        tokio::spawn(async move {
            debug!("Starting ConfigProvider");
            config_provider.run().instrument(provider_span).await;
        });

        let guard = server_gateway.clone();
        let gateway_span = Span::current();
        tokio::spawn(async move {
            debug!("Starting Gateway");
            guard
                .clone()
                .run(gateway_receiver)
                .instrument(gateway_span)
                .await;
        });
        let shared_clone = shared.clone();
        let registry_clone = shared_clone.filter_registry();

        tokio::spawn(async move {
            let config_clone = shared_clone.config();
            if let Some(filter_config) = &config_clone.filters {
                if let Some(rate_config) = &filter_config.rate_limiter {
                    let rate_limiter = RateLimiter::new(
                        "global_rate_limiter",
                        rate_config.request_limit,
                        rate_config.window_length,
                    );

                    if let Err(e) = registry_clone.register(rate_limiter).await {
                        debug!("Failed to register rate limiter: {}", e);
                    }
                }

                if config_clone.filters.as_ref().unwrap().ban.enabled {
                    match BanSystemAdapter::new(
                        "global_ban_system",
                        config_clone
                            .filters
                            .as_ref()
                            .unwrap()
                            .ban
                            .file_path
                            .as_ref()
                            .unwrap()
                            .clone(),
                    )
                    .await
                    {
                        Ok(ban_filter) => {
                            if let Err(e) = registry_clone.register(ban_filter).await {
                                debug!("Failed to register ban filter: {}", e);
                            }
                        }
                        Err(e) => {
                            error!("Failed to create ban system adapter: {}", e);
                        }
                    }
                }
            }
        });

        // Initialize system metrics collection if telemetry is enabled
        #[cfg(feature = "telemetry")]
        telemetry::start_system_metrics_collection();

        Ok(Self {
            shared,
            gateway: server_gateway,
        })
    }

    pub async fn run(self: Arc<Self>) -> Result<(), SendError> {
        debug!("Starting Infrarust server");
        let bind_addr = self.shared.config().bind.clone().unwrap_or_default();

        // Create listener
        let listener = match TcpListener::bind(&bind_addr).await {
            Ok(l) => l,
            Err(e) => {
                error!("Failed to bind to {}: {}", bind_addr, e);
                self.shared
                    .shutdown_controller()
                    .trigger_shutdown(&format!("Failed to bind: {}", e))
                    .await;
                return Err(SendError::new(e));
            }
        };

        info!("Listening on {}", bind_addr);

        // Get a shutdown receiver
        let mut shutdown_rx = self.shared.shutdown_controller().subscribe().await;
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    info!("Received shutdown signal, stopping server");
                    break;
                }

                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, addr)) => {
                            let self_guard = Arc::clone(&self);
                            let session_id = Uuid::new_v4();
                            let _span = debug_span!("TCP Connection", %addr, %session_id);
                            debug!("New TCP connection accepted : ({})[{}]", addr, session_id);

                            tokio::spawn(async move {
                                debug!("Starting connection processing for ({})[{}]", addr, session_id);

                                match async {

                                    self_guard.shared.filter_registry().filter(&stream).await?;


                                    debug!("Connection passed filters for ({})[{}]", addr, session_id);

                                    let conn = Connection::new(stream, session_id)
                                        .instrument(debug_span!("New connection"))
                                        .await?;
                                    debug!("Connection established for ({})[{}]", addr, session_id);

                                    self_guard.handle_connection(conn).await
                                }.await {
                                    Ok(_) => debug!("Connection from {} completed successfully", addr),
                                    Err(e) => error!("Connection error from {}: {}", addr, e),
                                }
                            });
                        }
                        Err(e) => {
                            error!("Accept error: {}", e);
                            if e.kind() == io::ErrorKind::Interrupted {
                                break;
                            }
                        }
                    }
                }
            }
        }

        info!("Server stopped accepting new connections");
        Ok(())
    }

    #[instrument(name = "connection_flow", skip(client))]
    async fn handle_connection(&self, mut client: Connection) -> io::Result<()> {
        let peer_addr = client.peer_addr().await?;
        Span::current().record("peer_addr", format!("{}", peer_addr));

        debug!(
            "Starting to process new connection from {}",
            client.peer_addr().await?
        );

        debug!("Reading handshake packet (with 10s timeout)");
        let handshake_packet = match client.read_packet().await {
            Ok(packet) => {
                debug!("Successfully read handshake packet");
                packet
            }
            Err(e) => {
                debug!("Failed to read handshake packet: {}", e);
                if let Err(close_err) = client.close().await {
                    warn!("Error closing client connection: {}", close_err);
                }
                return Err(e.into());
            }
        };

        debug!("Parsing handshake packet");
        let handshake = match ServerBoundHandshake::from_packet(&handshake_packet) {
            Ok(handshake) => {
                debug!("Successfully parsed handshake: {:?}", handshake);
                handshake
            }
            Err(e) => {
                debug!("Failed to parse handshake packet: {}", e);
                if let Err(close_err) = client.close().await {
                    warn!("Error closing client connection: {}", close_err);
                }
                return Err(io::Error::new(io::ErrorKind::InvalidData, e));
            }
        };

        let domain = handshake.parse_server_address();
        debug!(domain = %domain, "Processing connection for domain");

        debug!("Reading second packet (with 10s timeout)");
        let second_packet = match client.read_packet().await {
            Ok(packet) => {
                debug!("Successfully read second packet");
                packet
            }
            Err(e) => {
                debug!("Failed to read second packet: {}", e);
                if let Err(close_err) = client.close().await {
                    warn!("Error closing client connection: {}", close_err);
                }
                return Err(e.into());
            }
        };

        let protocol_version = Version::from(handshake.protocol_version.0);
        let is_login = handshake.is_login_request();

        debug!(
            domain = %domain,
            protocol_version = ?protocol_version,
            is_login = is_login,
            "Preparing server request"
        );

        let client_addr = client.peer_addr().await?;
        let session_id = client.session_id;

        let _handle_client_span = debug_span!(
            "handle_client_flow",
            domain = %domain,
            is_login = %is_login
        );
        let domain_clone = domain.clone();

        let gateway_ref = self.gateway.clone();
        tokio::spawn(async move {
            // let _guard = handle_client_span.entered();
            debug!("Processing client in separate task");

            gateway_ref
                .handle_client_connection(
                    client,
                    ServerRequest {
                        client_addr,
                        domain: domain.clone(),
                        is_login,
                        protocol_version,
                        read_packets: [handshake_packet, second_packet],
                        session_id,
                    },
                )
                .await;

            debug!(domain = %domain, "Client processing task completed");
        });

        debug!(domain = %domain_clone, "Connection handler completed");
        Ok(())
    }

    pub async fn shutdown(self: &Arc<Self>) -> tokio::sync::oneshot::Receiver<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();

        let self_guard = self.clone();
        tokio::spawn(async move {
            let config_service = self_guard.shared.configuration_service().clone();
            let configs = config_service.get_all_configurations().await;

            for (config_id, _) in configs {
                self_guard
                    .shared
                    .actor_supervisor()
                    .shutdown_actors(&config_id)
                    .await;
            }

            let _ = tx.send(());
        });

        rx
    }

    pub fn get_shared(&self) -> Arc<SharedComponent> {
        self.shared.clone()
    }

    pub async fn has_ban_filter(&self) -> Result<bool, FilterError> {
        let registry = self.shared.filter_registry();

        with_filter_or!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |_: &BanSystemAdapter| { Ok(true) },
            false
        )
    }

    pub async fn add_ban(&self, ban: BanEntry) -> Result<(), FilterError> {
        let registry = self.shared.filter_registry();

        with_filter!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| { filter.add_ban(ban).await }
        )
    }

    pub async fn remove_ban_by_ip(&self, ip: IpAddr) -> Result<bool, FilterError> {
        let registry = self.shared.filter_registry();
        with_filter!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| { filter.remove_ban_by_ip(&ip, "system").await }
        )
    }

    pub async fn remove_ban_by_username(&self, username: &str) -> Result<bool, FilterError> {
        let registry = self.shared.filter_registry();

        with_filter!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| {
                filter.remove_ban_by_username(username, "system").await
            }
        )
    }

    pub async fn remove_ban_by_uuid(&self, uuid: &str) -> Result<bool, FilterError> {
        let registry = self.shared.filter_registry();

        with_filter!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| { filter.remove_ban_by_uuid(uuid, "system").await }
        )
    }

    pub async fn get_all_bans(&self) -> Result<Vec<BanEntry>, FilterError> {
        let registry = self.shared.filter_registry();

        with_filter!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| { filter.get_all_bans().await }
        )
    }

    pub async fn clear_expired_bans(&self) -> Result<usize, FilterError> {
        let registry = self.shared.filter_registry();

        with_filter!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| { filter.clear_expired_bans().await }
        )
    }

    pub async fn get_ban_file_path(&self) -> String {
        self.shared
            .config()
            .filters
            .clone()
            .unwrap()
            .ban
            .file_path
            .unwrap()
    }

    pub async fn has_ban_system_adapter(&self) -> Result<bool, FilterError> {
        let registry = self.shared.filter_registry();

        with_filter_or!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |_: &BanSystemAdapter| { Ok(true) },
            false
        )
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore = "TODO"]
    async fn test_infrared_basic() {
        // TODO: Add integration tests that simulate client connections
    }
}
