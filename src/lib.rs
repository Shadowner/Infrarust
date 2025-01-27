//! Infrarust - A Minecraft proxy server implementation in Rust
//!
//! This crate provides a flexible and efficient proxy server for Minecraft,
//! supporting multiple backend servers, protocol versions, and various security features.
//! It's designed to proxy multiple domain names to different type of Minecraft servers

// Core modules
pub mod core;
use core::config::provider::file::{FileProvider, FileType};
use core::config::provider::ConfigProvider;
pub use core::config::InfrarustConfig;
pub use core::error::RsaError;
use core::error::SendError;
use core::event::{GatewayMessage, ProviderMessage};
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
use proxy_modes::passthrough::PassthroughMode;
pub use security::{
    encryption::EncryptionState,
    filter::{Filter, FilterChain, FilterConfig},
    rate_limiter::RateLimiter,
};

// Server implementation
pub mod server;

use serde::de;
use server::gateway::Gateway;
use server::{ServerRequest, ServerRequester, ServerResponse};
use tokio::net::TcpListener;
use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex;

use crate::version::Version;

pub struct Infrarust {
    config: InfrarustConfig,
    filter_chain: FilterChain,
    //TODO: For future use
    _connections: Arc<Mutex<HashMap<String, Connection>>>,
    server_gateway: Arc<Gateway>,

    gateway_sender: Sender<GatewayMessage>,
    provider_sender: Sender<ProviderMessage>,
}

impl Infrarust {
    pub fn new(config: InfrarustConfig) -> io::Result<Self> {
        let (gateway_sender, gateway_receiver) = tokio::sync::mpsc::channel(100);
        let (provider_sender, provider_receiver) = tokio::sync::mpsc::channel(100);

        let server_gateway = Arc::new(Gateway::new(gateway_sender.clone()));
        let mut config_provider = ConfigProvider::new(
            gateway_sender.clone(),
            provider_receiver,
            provider_sender.clone(),
        );

        let file_provider = FileProvider::new(
            vec![
                "./proxies"
                    .to_string(),
            ],
            FileType::Yaml,
            true,
            provider_sender.clone(),
        );

        let guard = server_gateway.clone();
        tokio::spawn(async move {
            debug!("Starting Gateway");
            guard.clone().run(gateway_receiver).await;
        });

        tokio::spawn(async move {
            debug!("Starting ConfigProvider");
            config_provider.register_provider(Box::new(file_provider));
            config_provider.run().await;
        });

        Ok(Self {
            config: config.clone(),
            filter_chain: FilterChain::new(),
            _connections: Arc::new(Mutex::new(HashMap::new())),
            server_gateway,
            gateway_sender,
            provider_sender,
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
                    let this = Arc::clone(&self);

                    tokio::spawn(async move {
                        if let Err(e) = async move {
                            let filter_chain = this.filter_chain.clone();
                            let server_gateway = Arc::clone(&this.server_gateway);

                            filter_chain.filter(&stream).await?;
                            let conn = Connection::new(stream).await?;
                            Self::handle_connection(this, conn, server_gateway).await?;

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
        this: Arc<Self>,
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

        // let response = match server_gateway
        //     .request_server(ServerRequest {
        //         client_addr: client.peer_addr().await?,
        //         domain: domain.clone(),
        //         is_login: handshake.is_login_request(),
        //         protocol_version,
        //         read_packets: [handshake_packet, second_packet],
        //     })
        //     .await
        // {
        //     Ok(resp) => resp,
        //     Err(e) => {
        //         error!("Failed to find server for domain '{}': {}", domain, e);
        //         return Err(io::Error::new(io::ErrorKind::NotFound, e.to_string()));
        //     }
        // };

        // if handshake.is_status_request() {
        //     debug!("Handling status request for domain: {}", domain);
        //     return Self::handle_status(client, response).await;
        // }

        // debug!("Handling login request for domain: {}", domain);
        // Self::handle_login(this, client, response, protocol_version).await
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

    // async fn handle_login(
    //     this: Arc<Self>,
    //     client: Connection,
    //     response: ServerResponse,
    //     protocol_version: Version,
    // ) -> io::Result<()> {
    //     let proxy_mode: Box<dyn ProxyModeHandler> = match response.proxy_mode {
    //         ProxyModeEnum::Passthrough => Box::new(PassthroughMode),
    //         ProxyModeEnum::Full => Box::new(FullMode),
    //         ProxyModeEnum::ClientOnly => Box::new(ClientOnlyMode),
    //         ProxyModeEnum::Offline => Box::new(OfflineMode),
    //         ProxyModeEnum::ServerOnly => {
    //             return Err(io::Error::new(
    //                 io::ErrorKind::Other,
    //                 "Server-only mode not supported yet",
    //             ))
    //         }
    //     };
    //     let server_gateway = Arc::clone(&this.server_gateway);

    //     let login_start = &response.read_packets[1];
    //     let username = ServerBoundLoginStart::try_from(login_start)?.name.0;
    //     let server_addr = response.server_addr;
    //     info!(
    //         "Handling login request for user: {} on {}({}) in ProxyMode : {:?}",
    //         username,
    //         response
    //             .proxied_domain
    //             .clone()
    //             .unwrap_or("Direct IP".to_string()),
    //         server_addr
    //             .map(|addr| addr.to_string())
    //             .unwrap_or_else(|| "Unknown".to_string()),
    //         response.proxy_mode
    //     );

    //     debug!(
    //         "Handling login request with proxy mode: {:?}",
    //         response.proxy_mode
    //     );

    //     server_gateway
    //         .handle_full_connection(client, response)
    //         .await;
    //     Ok(())
    // }
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
