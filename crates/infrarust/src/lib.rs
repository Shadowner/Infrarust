//! Infrarust - A Minecraft proxy server implementation in Rust
//!
//! This crate provides a flexible and efficient proxy server for Minecraft,
//! supporting multiple backend servers, protocol versions, and various security features.
//! It's designed to proxy multiple domain names to different type of Minecraft servers

// Core modules
pub mod core;
use core::actors::supervisor::ActorSupervisor;
use core::config::provider::ConfigProvider;
use core::config::service::ConfigurationService;
pub use core::error::RsaError;
use core::error::SendError;
use core::shared_component::SharedComponent;
use std::io;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

pub mod telemetry;

use infrarust_ban_system::BanEntry;
use infrarust_config::{
    models::{logging::LogType, manager::{ManagerConfig, PterodactylManagerConfig}}, provider::{docker::DockerProvider, file::FileProvider}, InfrarustConfig
};
use infrarust_protocol::minecraft::java::handshake::ServerBoundHandshake;
use infrarust_protocol::version::Version;
use infrarust_server_manager::{LocalProvider, PterodactylClient};
use security::filter::FilterError;
use server::manager::Manager;
use tracing::{Instrument, Span, debug, debug_span, error, info, instrument, warn};

// Network and security modules
pub mod network;
pub mod security;
pub use network::proxy_protocol::reader::ProxyProtocolReader;
pub use network::{
    connection::{Connection, ServerConnection},
    proxy_protocol::write_proxy_protocol_header,
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
        let span = debug_span!("infrarust_init", log_type = LogType::Supervisor.as_str());
        let _enter = span.enter();
        let config = Arc::new(config);

        debug!(log_type = LogType::Supervisor.as_str(), "Initializing Infrarust server with config: {:?}", config);
        let config_service = Arc::new(ConfigurationService::new());

        let (gateway_sender, gateway_receiver) = tokio::sync::mpsc::channel(100);
        let (provider_sender, provider_receiver) = tokio::sync::mpsc::channel(100);

        // Initialize filter registry
        let filter_registry = Arc::new(FilterRegistry::new());

        let mut config_provider = ConfigProvider::new(
            config_service.clone(),
            provider_receiver,
            provider_sender.clone(),
        );

        let manager_config = config
            .managers_config
            .clone()
            .unwrap_or(ManagerConfig { pterodactyl: None });

        let pterodactyl_config = match manager_config.pterodactyl {
            Some(ref config) => config.clone(),
            None => {
                error!(log_type = LogType::Supervisor.as_str(), "Pterodactyl manager configuration is missing");
                PterodactylManagerConfig {
                    enabled: false,
                    api_key: String::new(),
                    base_url: String::new(),
                }
            }
        };

        debug!(
            log_type = LogType::ServerManager.as_str(),
            "Pterodactyl manager configuration: enabled = {}, api_key = {}, base_url = {}",
            pterodactyl_config.enabled, pterodactyl_config.api_key, pterodactyl_config.base_url
        );

        let pterodactyl_provider =
            PterodactylClient::new(pterodactyl_config.api_key, pterodactyl_config.base_url);
        let local_provider = LocalProvider::new();

        let managers = Arc::new(Manager::new(pterodactyl_provider, local_provider));

        if ActorSupervisor::initialize_global(Some(managers.clone())).is_err() {
            error!(log_type = LogType::Supervisor.as_str(), "Failed to initialize ActorSupervisor");
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Failed to initialize ActorSupervisor",
            ));
        }
        let supervisor = ActorSupervisor::global();

        let shared = Arc::new(SharedComponent::new(
            config,
            supervisor.clone(),
            config_service,
            filter_registry,
            shutdown_controller,
            gateway_sender,
            provider_sender,
            managers.clone(),
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
            let docker_provider = Box::new(DockerProvider::new(
                docker_config,
                shared.provider_sender().clone(),
            ));
            config_provider.register_provider(docker_provider);
            info!("Docker provider registered");
        }

        let provider_span = Span::current();
        tokio::spawn(async move {
            debug!(log_type = LogType::ConfigProvider.as_str(), "Starting ConfigProvider");
            config_provider.run().instrument(provider_span).await;
        });

        let guard = server_gateway.clone();
        let gateway_span = Span::current();
        tokio::spawn(async move {
            debug!(log_type = LogType::Supervisor.as_str(), "Starting Gateway");
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
                    // REFACTO : Fix this
                    let rate_limiter = RateLimiter::new(
                        "global_rate_limiter",
                        rate_config.burst_size,
                        Duration::from_secs(60),
                    );

                    if let Err(e) = registry_clone.register(rate_limiter).await {
                        debug!(log_type = LogType::Filter.as_str(), "Failed to register rate limiter: {}", e);
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
                                debug!(log_type = LogType::BanSystem.as_str(), "Failed to register ban filter: {}", e);
                            }
                        }
                        Err(e) => {
                            error!(log_type = LogType::BanSystem.as_str(), "Failed to create ban system adapter: {}", e);
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
        debug!(log_type = LogType::Supervisor.as_str(), "Starting Infrarust server");
        let bind_addr = self.shared.config().bind.clone().unwrap_or_default();

        // Create listener
        let listener = match TcpListener::bind(&bind_addr).await {
            Ok(l) => l,
            Err(e) => {
                error!(log_type = LogType::TcpConnection.as_str(), "Failed to bind to {}: {}", bind_addr, e);
                self.shared
                    .shutdown_controller()
                    .trigger_shutdown(&format!("Failed to bind: {}", e))
                    .await;
                return Err(SendError::new(e));
            }
        };

        info!(log_type = LogType::TcpConnection.as_str(), "Listening on {}", bind_addr);

        // Get a shutdown receiver
        let mut shutdown_rx = self.shared.shutdown_controller().subscribe().await;
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    info!(log_type = LogType::Supervisor.as_str(), "Received shutdown signal, stopping server");
                    break;
                }

                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, addr)) => {
                            let self_guard = Arc::clone(&self);
                            let session_id = Uuid::new_v4();
                            let _span = debug_span!("TCP Connection", %addr, %session_id, log_type = LogType::TcpConnection.as_str());
                            debug!(log_type = LogType::TcpConnection.as_str(), "New TCP connection accepted : ({})[{}]", addr, session_id);

                            tokio::spawn(async move {
                                debug!(log_type = LogType::TcpConnection.as_str(), "Starting connection processing for ({})[{}]", addr, session_id);

                                match async {

                                    self_guard.shared.filter_registry().filter(&stream).await?;


                                    debug!(log_type = LogType::Filter.as_str(), "Connection passed filters for ({})[{}]", addr, session_id);

                                 let conn = if self_guard.shared.config().proxy_protocol.is_some() {
                                        debug!(log_type = LogType::ProxyProtocol.as_str(), "Using proxy protocol sent by client");
                                        Connection::with_proxy_protocol(
                                            stream,
                                            session_id,
                                            self_guard.shared.config().proxy_protocol.as_ref()
                                        )
                                        .instrument(debug_span!("New connection with proxy protocol", log_type = LogType::ProxyProtocol.as_str()))
                                        .await?
                                    } else {
                                        debug!(log_type = LogType::TcpConnection.as_str(), "Not using proxy protocol");
                                        Connection::new(stream, session_id)
                                            .instrument(debug_span!("New connection", log_type = LogType::TcpConnection.as_str()))
                                            .await?
                                    };
                                    debug!(log_type = LogType::TcpConnection.as_str(), "Connection established for ({})[{}]", addr, session_id);

                                    self_guard.handle_connection(conn).await
                                }.await {
                                    Ok(_) => debug!(log_type = LogType::TcpConnection.as_str(), "Connection from {} completed successfully", addr),
                                    Err(e) => error!(log_type = LogType::TcpConnection.as_str(), "Connection error from {}: {}", addr, e),
                                }
                            });
                        }
                        Err(e) => {
                            error!(log_type = LogType::TcpConnection.as_str(), "Accept error: {}", e);
                            if e.kind() == io::ErrorKind::Interrupted {
                                break;
                            }
                        }
                    }
                }
            }
        }

        info!(log_type = LogType::Supervisor.as_str(), "Server stopped accepting new connections");
        Ok(())
    }

    #[instrument(name = "connection_flow", skip(client), fields(log_type = LogType::TcpConnection.as_str()))]
    async fn handle_connection(&self, mut client: Connection) -> io::Result<()> {
        let peer_addr = client.peer_addr().await?;
        Span::current().record("peer_addr", format!("{}", peer_addr));

        debug!(
            log_type = LogType::TcpConnection.as_str(),
            "Starting to process new connection from {}",
            client.peer_addr().await?
        );

        debug!(log_type = LogType::PacketProcessing.as_str(), "Reading handshake packet (with 10s timeout)");
        let handshake_packet = match client.read_packet().await {
            Ok(packet) => {
                debug!(log_type = LogType::PacketProcessing.as_str(), "Successfully read handshake packet");
                packet
            }
            Err(e) => {
                debug!(log_type = LogType::PacketProcessing.as_str(), "Failed to read handshake packet: {}", e);
                if let Err(close_err) = client.close().await {
                    warn!(log_type = LogType::TcpConnection.as_str(), "Error closing client connection: {}", close_err);
                }
                return Err(e.into());
            }
        };

        debug!(log_type = LogType::PacketProcessing.as_str(), "Parsing handshake packet");
        let handshake = match ServerBoundHandshake::from_packet(&handshake_packet) {
            Ok(handshake) => {
                debug!(log_type = LogType::PacketProcessing.as_str(), "Successfully parsed handshake: {:?}", handshake);
                handshake
            }
            Err(e) => {
                debug!(log_type = LogType::PacketProcessing.as_str(), "Failed to parse handshake packet: {}", e);
                if let Err(close_err) = client.close().await {
                    warn!(log_type = LogType::TcpConnection.as_str(), "Error closing client connection: {}", close_err);
                }
                return Err(io::Error::new(io::ErrorKind::InvalidData, e));
            }
        };

        let domain = handshake.parse_server_address();
        debug!(domain = %domain, log_type = LogType::PacketProcessing.as_str(), "Processing connection for domain");

        debug!(log_type = LogType::PacketProcessing.as_str(), "Reading second packet (with 10s timeout)");
        let second_packet = match client.read_packet().await {
            Ok(packet) => {
                debug!(log_type = LogType::PacketProcessing.as_str(), "Successfully read second packet");
                packet
            }
            Err(e) => {
                debug!(log_type = LogType::PacketProcessing.as_str(), "Failed to read second packet: {}", e);
                if let Err(close_err) = client.close().await {
                    warn!(log_type = LogType::TcpConnection.as_str(), "Error closing client connection: {}", close_err);
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
            log_type = LogType::ServerManager.as_str(),
            "Preparing server request"
        );

        let client_addr = client.peer_addr().await?;
        let session_id = client.session_id;
        let original_client_addr = client.original_client_addr;

        let _handle_client_span = debug_span!(
            "handle_client_flow",
            domain = %domain,
            is_login = %is_login,
            log_type = LogType::ServerManager.as_str()
        );
        let domain_clone = domain.clone();

        let gateway_ref = self.gateway.clone();
        tokio::spawn(async move {
            // let _guard = handle_client_span.entered();
            debug!(log_type = LogType::ServerManager.as_str(), "Processing client in separate task");

            if let Some(original_addr) = &original_client_addr {
                debug!(
                    log_type = LogType::ProxyProtocol.as_str(),
                    "Using original client address from proxy protocol: {}",
                    original_addr
                );
            }

            gateway_ref
                .handle_client_connection(
                    client,
                    ServerRequest {
                        client_addr,
                        original_client_addr,
                        domain: domain.clone(),
                        is_login,
                        protocol_version,
                        read_packets: [handshake_packet, second_packet],

                        session_id,
                    },
                )
                .await;

            debug!(domain = %domain, log_type = LogType::ServerManager.as_str(), "Client processing task completed");
        });

        debug!(domain = %domain_clone, log_type = LogType::TcpConnection.as_str(), "Connection handler completed");
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
