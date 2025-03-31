//! Infrarust - A Minecraft proxy server implementation in Rust
//!
//! This crate provides a flexible and efficient proxy server for Minecraft,
//! supporting multiple backend servers, protocol versions, and various security features.
//! It's designed to proxy multiple domain names to different type of Minecraft servers

// Core modules
pub mod core;
use core::config::provider::file::FileProvider;
use core::config::provider::ConfigProvider;
use core::config::service::ConfigurationService;
pub use core::config::InfrarustConfig;
pub use core::error::RsaError;
use core::error::SendError;
use core::event::{GatewayMessage, ProviderMessage};
use std::io;
use std::sync::Arc;

pub mod telemetry;

// Protocol modules
pub mod protocol;
use lazy_static::lazy_static;
use parking_lot::RwLock;
use protocol::minecraft::java::handshake::ServerBoundHandshake;
pub use protocol::{
    types::{ProtocolRead, ProtocolWrite},
    version,
};
use tracing::{debug, debug_span, error, info, instrument, Instrument, Span}; // Remplacer log par tracing

// Network and security modules
pub mod network;
pub mod security;
pub use network::{
    connection::{Connection, ServerConnection},
    proxy_protocol::{write_proxy_protocol_header, ProxyProtocolConfig},
};
pub mod proxy_modes;
pub use security::{
    encryption::EncryptionState,
    filter::{Filter, FilterChain, FilterConfig},
    rate_limiter::RateLimiter,
};

// Server implementation
pub mod server;

use server::gateway::Gateway;
use server::ServerRequest;
use tokio::net::TcpListener;
use tokio::sync::mpsc::Sender;
use uuid::Uuid;

use crate::version::Version;

pub struct Infrarust {
    _config_service: Arc<ConfigurationService>,
    config: InfrarustConfig,
    filter_chain: FilterChain,
    server_gateway: Arc<Gateway>,

    _gateway_sender: Sender<GatewayMessage>,
    _provider_sender: Sender<ProviderMessage>,
}

lazy_static! {
    pub static ref CONFIG: RwLock<Arc<InfrarustConfig>> =
        RwLock::new(Arc::new(InfrarustConfig::default()));
}

impl Infrarust {
    pub fn new(config: InfrarustConfig) -> io::Result<Self> {
        let span = debug_span!("infrarust_init");
        let _enter = span.enter();

        debug!("Initializing Infrarust server with config: {:?}", config);
        let config_service = Arc::new(ConfigurationService::new());
        {
            let mut guard = CONFIG.write();
            *guard = Arc::new(config.clone());
        }
        let (gateway_sender, gateway_receiver) = tokio::sync::mpsc::channel(100);
        let (provider_sender, provider_receiver) = tokio::sync::mpsc::channel(100);

        let server_gateway = Arc::new(Gateway::new(gateway_sender.clone(), config_service.clone()));

        let mut config_provider = ConfigProvider::new(
            config_service.clone(),
            provider_receiver,
            provider_sender.clone(),
        );

        if let Some(file_config) = config.file_provider.clone() {
            let file_provider = FileProvider::new(
                file_config.proxies_path,
                file_config.file_type,
                file_config.watch,
                provider_sender.clone(),
            );

            config_provider.register_provider(Box::new(file_provider));
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

        Ok(Self {
            _config_service: config_service.clone(),
            config,
            filter_chain: FilterChain::new(),
            server_gateway,
            _gateway_sender: gateway_sender,
            _provider_sender: provider_sender,
        })
    }

    pub async fn run(self: Arc<Self>) -> Result<(), SendError> {
        debug!("Starting Infrarust server");
        let bind_addr = self.config.bind.clone().unwrap_or_default();
        let listener = TcpListener::bind(&bind_addr).await?;
        info!("Listening on {}", bind_addr);

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let session_id = Uuid::new_v4();
                    let span = debug_span!("TCP Connection", %addr, %session_id);
                    debug!("New TCP connection accepted : ({})[{}]", addr, session_id);

                    let filter_chain = self.filter_chain.clone();
                    let server_gateway = Arc::clone(&self.server_gateway);
                    let clone_span = span.clone();
                    tokio::spawn(async move {
                        if let Err(e) = async move {
                            debug!("Starting connection processing for ({})[{}]", addr, session_id);
                            filter_chain.filter(&stream).await?;
                            debug!("Connection passed filters for ({})[{}]", addr, session_id);
                            let conn = Connection::new(stream, session_id)
                                .instrument(debug_span!("New connection"))
                                .await?;
                            debug!("Connection established for ({})[{}]", addr, session_id);
                            Self::handle_connection(conn, server_gateway)
                                .instrument(clone_span)
                                .await?;

                            Ok::<_, SendError>(())
                        }
                        .instrument(span)
                        .await
                        {
                            error!("Connection error from {}: {}", addr, e);
                        }
                    })
                    .await
                    .unwrap_or_default();
                }
                Err(e) => {
                    error!("Accept error: {}", e);
                }
            }
        }
    }

    #[instrument(name = "connection_flow", skip(client, server_gateway), fields(
        peer_addr = %client.peer_addr().await?,
    ))]
    async fn handle_connection(
        mut client: Connection,
        server_gateway: Arc<Gateway>,
    ) -> io::Result<()> {

        let handshake_packet = if let Ok(packet) = client.read_packet().await {
            packet
        } else {
            debug!("Failed to read handshake packet");
            return Ok(());
        };
        
        let handshake = ServerBoundHandshake::from_packet(&handshake_packet)?;
        let domain = handshake.parse_server_address();

        debug!(domain = %domain, "Received connection request");

        let second_packet = client.read_packet().await?;
        let protocol_version = Version::from(handshake.protocol_version.0);
        let is_login = handshake.is_login_request();

        let client_addr = client.peer_addr().await?;
        let session_id = client.session_id;
        Gateway::handle_client_connection(
            client,
            ServerRequest {
                client_addr,
                domain: domain.clone(),
                is_login,
                protocol_version,
                read_packets: [handshake_packet, second_packet],
                session_id,
            },
            server_gateway,
        )
        .instrument(debug_span!("handle_client_flow", domain = %domain, is_login = %is_login))
        .await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    #[ignore = "TODO"]
    async fn test_infrared_basic() {
        let config = InfrarustConfig {
            bind: Some("127.0.0.1:25565".to_string()),
            keepalive_timeout: Some(Duration::from_secs(30)),
            ..Default::default()
        };

        let infrared = Arc::new(Infrarust::new(config).unwrap());

        tokio::spawn(async move {
            infrared.run().await.unwrap();
        });

        // TODO: Add integration tests that simulate client connections
    }
}
