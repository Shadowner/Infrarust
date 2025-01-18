//! Infrarust - A Minecraft proxy server implementation in Rust
//!
//! This crate provides a flexible and efficient proxy server for Minecraft,
//! supporting multiple backend servers, protocol versions, and various security features.
//! It's designed to proxy multiple domain names to different type of Minecraft servers

// Core modules
pub mod core;
pub use core::config::{FileProvider, FileType, InfrarustConfig};
pub use core::error::RsaError;
use core::error::SendError;
use std::collections::HashMap;
use std::io;
use std::sync::Arc;

// Protocol modules
pub mod protocol;
use log::{debug, error, info};
use network::packet::{Packet, PacketCodec};
use protocol::minecraft::java::handshake::ServerBoundHandshake;
use protocol::minecraft::java::login::ServerBoundLoginStart;
use protocol::minecraft::java::status::clientbound_response::{
    ClientBoundResponse, PlayersJSON, ResponseJSON, VersionJSON, CLIENTBOUND_RESPONSE_ID,
};
use protocol::types::ProtocolString;
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
use proxy_modes::client_only::ClientOnlyMode;
use proxy_modes::full::FullMode;
use proxy_modes::offline::OfflineMode;
use proxy_modes::passthrough::PassthroughMode;
use proxy_modes::{ProxyModeEnum, ProxyModeHandler};
pub use security::{
    encryption::EncryptionState,
    filter::{Filter, FilterChain, FilterConfig},
    rate_limiter::RateLimiter,
};

// Server implementation
pub mod server;

use server::gateway::ServerGateway;
use server::{ServerRequest, ServerRequester, ServerResponse};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::version::Version;

pub struct Infrarust {
    config: InfrarustConfig,
    filter_chain: FilterChain,
    //TODO: For future use
    _connections: Arc<Mutex<HashMap<String, Connection>>>,
    server_gateway: Arc<ServerGateway>,
}

impl Infrarust {
    pub fn new(config: InfrarustConfig) -> io::Result<Self> {
        let server_gateway = Arc::new(ServerGateway::new(config.server_configs.clone()));

        Ok(Self {
            config: config.clone(),
            filter_chain: FilterChain::new(),
            _connections: Arc::new(Mutex::new(HashMap::new())),
            server_gateway,
        })
    }

    pub async fn run(self: Arc<Self>) -> Result<(), SendError> {
        let bind_addr = self.config.bind.clone().unwrap_or_default();
        let listener = TcpListener::bind(&bind_addr).await?;
        info!("Listening on {}", bind_addr);

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let this = Arc::clone(&self);

                    tokio::spawn(async move {
                        if let Err(e) = async move {
                            let filter_chain = this.filter_chain.clone();
                            let server_gateway = Arc::clone(&this.server_gateway);

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
        server_gateway: Arc<ServerGateway>,
    ) -> io::Result<()> {
        let handshake_packet = client.read_packet().await?;
        let handshake = ServerBoundHandshake::from_packet(&handshake_packet)?;
        let domain = handshake.parse_server_address();

        debug!("Received connection request for domain: {}", domain);
        debug!("Handshake packet: {:?}", handshake);

        let second_packet = client.read_packet().await?;
        let protocol_version = Version::from(handshake.protocol_version.0);
        let response = match server_gateway
            .request_server(ServerRequest {
                client_addr: client.peer_addr().await?,
                domain: domain.clone(),
                is_login: handshake.is_login_request(),
                protocol_version,
                read_packets: [handshake_packet, second_packet],
            })
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                error!("Failed to find server for domain '{}': {}", domain, e);
                return Err(io::Error::new(io::ErrorKind::NotFound, e.to_string()));
            }
        };

        if handshake.is_status_request() {
            debug!("Handling status request for domain: {}", domain);
            return Self::handle_status(client, response).await;
        }

        debug!("Handling login request for domain: {}", domain);
        Self::handle_login(client, response, protocol_version).await
    }

    async fn handle_status(mut client: Connection, mut response: ServerResponse) -> io::Result<()> {
        debug!("Handling status request");

        if response.status_response.is_none() {
            let status_json = ResponseJSON {
                version: VersionJSON {
                    name: "1.18.2".to_string(),
                    protocol: 758,
                },
                players: PlayersJSON {
                    max: 100,
                    online: 0,
                    sample: vec![],
                },
                description: serde_json::json!({
                    "text": "Minecraft Server"
                }),
                favicon: None,
                previews_chat: false,
                enforces_secure_chat: false,
                modinfo: None,
                forge_data: None,
            };

            let json_str = serde_json::to_string(&status_json)?;
            let mut response_packet = Packet::new(CLIENTBOUND_RESPONSE_ID);
            response_packet.encode(&ClientBoundResponse {
                json_response: ProtocolString(json_str),
            })?;
            response.status_response = Some(response_packet);
        }

        if let Some(status) = response.status_response {
            debug!("Sending status response");
            client.write_packet(&status).await?;
        }

        debug!("Waiting for ping packet");
        match client.read_packet().await {
            Ok(ping_packet) => {
                debug!("Received ping packet, sending response");
                client.write_packet(&ping_packet).await?;
            }
            Err(e) => {
                error!("Error reading ping packet: {}", e);
                return Err(e.into());
            }
        }

        Ok(())
    }

    async fn handle_login(
        client: Connection,
        response: ServerResponse,
        protocol_version: Version,
    ) -> io::Result<()> {
        let proxy_mode: Box<dyn ProxyModeHandler> = match response.proxy_mode {
            ProxyModeEnum::Passthrough => Box::new(PassthroughMode),
            ProxyModeEnum::Full => Box::new(FullMode),
            ProxyModeEnum::ClientOnly => Box::new(ClientOnlyMode),
            ProxyModeEnum::Offline => Box::new(OfflineMode),
            ProxyModeEnum::ServerOnly => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Server-only mode not supported yet",
                ))
            }
        };

        let login_start = &response.read_packets[1];
        let username = ServerBoundLoginStart::try_from(login_start)?.name.0;
        let server_addr = response.server_addr;
        info!(
            "Handling login request for user: {} on {}({}) in ProxyMode : {:?}",
            username,
            response
                .proxied_domain
                .clone()
                .unwrap_or("Direct IP".to_string()),
            server_addr
                .map(|addr| addr.to_string())
                .unwrap_or_else(|| "Unknown".to_string()),
            response.proxy_mode
        );

        debug!(
            "Handling login request with proxy mode: {:?}",
            response.proxy_mode
        );
        proxy_mode.handle(client, response, protocol_version).await
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
            server_configs: vec![],
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
