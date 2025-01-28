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

// Protocol modules
pub mod protocol;
use log::{debug, error, info};
use protocol::minecraft::java::handshake::ServerBoundHandshake;
pub use protocol::{
    types::{ProtocolRead, ProtocolWrite},
    version,
};

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

use crate::version::Version;

pub struct Infrarust {
    _config_service: Arc<ConfigurationService>,
    config: InfrarustConfig,
    filter_chain: FilterChain,
    server_gateway: Arc<Gateway>,

    _gateway_sender: Sender<GatewayMessage>,
    _provider_sender: Sender<ProviderMessage>,
}

impl Infrarust {
    pub fn new(config: InfrarustConfig) -> io::Result<Self> {
        let config_service = Arc::new(ConfigurationService::new());

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

        tokio::spawn(async move {
            debug!("Starting ConfigProvider");
            config_provider.run().await;
        });

        let guard = server_gateway.clone();
        tokio::spawn(async move {
            debug!("Starting Gateway");
            guard.clone().run(gateway_receiver).await;
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
                    let filter_chain = self.filter_chain.clone();
                    let server_gateway = Arc::clone(&self.server_gateway);

                    tokio::spawn(async move {
                        if let Err(e) = async move {
                            filter_chain.filter(&stream).await?;
                            let conn = Connection::new(stream).await?;
                            Self::handle_connection(conn, server_gateway).await?;

                            Ok::<_, SendError>(())
                        }
                        .await
                        {
                            error!("Connection error from {}: {}", addr, e);
                        }
                    });
                }
                Err(e) => {
                    error!("Accept error: {}", e);
                }
            }
        }
    }

    async fn handle_connection(
        mut client: Connection,
        server_gateway: Arc<Gateway>,
    ) -> io::Result<()> {
        let handshake_packet = client.read_packet().await?;
        let handshake = ServerBoundHandshake::from_packet(&handshake_packet)?;
        let domain = handshake.parse_server_address();

        debug!("Received connection request for domain: {}", domain);
        debug!("Handshake packet: {:?}", handshake);

        let second_packet = client.read_packet().await?;
        let protocol_version = Version::from(handshake.protocol_version.0);

        let client_addr = client.peer_addr().await?;
        tokio::spawn(async move {
            Gateway::handle_client_connection(
                client,
                ServerRequest {
                    client_addr: client_addr,
                    domain: domain.clone(),
                    is_login: handshake.is_login_request(),
                    protocol_version,
                    read_packets: [handshake_packet, second_packet],
                },
                server_gateway,
            )
            .await;
        });

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
