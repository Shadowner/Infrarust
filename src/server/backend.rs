use std::{net::SocketAddr, sync::Arc};

use tokio::net::TcpStream;
use tracing::debug;
use uuid::Uuid;

use crate::{
    ProxyProtocolConfig, ServerConnection,
    core::config::ServerConfig,
    network::proxy_protocol::{ProtocolResult, errors::ProxyProtocolError},
    telemetry::TELEMETRY,
    write_proxy_protocol_header,
};

#[derive(Clone)]
pub struct Server {
    pub config: Arc<ServerConfig>,
}

impl Server {
    pub fn new(config: Arc<ServerConfig>) -> ProtocolResult<Self> {
        if config.addresses.is_empty() {
            return Err(ProxyProtocolError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "No server addresses configured",
            )));
        }
        Ok(Self { config })
    }

    pub async fn dial(&self, session_id: Uuid) -> ProtocolResult<ServerConnection> {
        let mut last_error = None;
        debug!("Dialing server with ping: {:?}", self.config.addresses);

        for addr in &self.config.addresses {
            let now = std::time::Instant::now();
            TELEMETRY.record_backend_request_start(&self.config.config_id, addr, &session_id);
            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    debug!("Connected to {}", addr);
                    stream.set_nodelay(true)?;
                    TELEMETRY.record_backend_request_end(
                        &self.config.config_id,
                        addr,
                        now,
                        true,
                        &session_id,
                        None,
                    );
                    return Ok(ServerConnection::new(stream, session_id).await?);
                }
                Err(e) => {
                    debug!("Failed to connect to {}: {}", addr, e);
                    TELEMETRY.record_backend_request_end(
                        &self.config.config_id,
                        addr,
                        now,
                        false,
                        &session_id,
                        Some(&e),
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap().into())
    }

    pub async fn dial_with_proxy_protocol(
        &self,
        session_id: Uuid,
        client_addr: SocketAddr,
    ) -> ProtocolResult<ServerConnection> {
        let mut last_error = None;
        debug!(
            "Dialing server with proxy protocol: {:?}",
            self.config.addresses
        );

        for addr in &self.config.addresses {
            let now = std::time::Instant::now();
            TELEMETRY.record_backend_request_start(&self.config.config_id, addr, &session_id);

            match TcpStream::connect(addr).await {
                Ok(mut stream) => {
                    debug!("Connected to {}", addr);
                    stream.set_nodelay(true)?;

                    // Envoyer l'en-tête du Proxy Protocol si activé
                    if self.config.send_proxy_protocol.unwrap_or(false) {
                        let server_sock_addr = stream.local_addr()?;

                        let proxy_config = ProxyProtocolConfig {
                            enabled: true,
                            version: self.config.proxy_protocol_version, //TODO: ServerConfig
                        };

                        match write_proxy_protocol_header(
                            &mut stream,
                            client_addr,
                            server_sock_addr,
                            &proxy_config,
                        )
                        .await
                        {
                            Ok(_) => debug!("Proxy protocol header sent"),
                            Err(e) => {
                                debug!("Failed to write proxy protocol header: {}", e);
                                TELEMETRY.record_backend_request_end(
                                    &self.config.config_id,
                                    addr,
                                    now,
                                    false,
                                    &session_id,
                                    Some(&e),
                                );
                                last_error = Some(e);
                                continue;
                            }
                        }
                    }

                    TELEMETRY.record_backend_request_end(
                        &self.config.config_id,
                        addr,
                        now,
                        true,
                        &session_id,
                        None,
                    );

                    return Ok(ServerConnection::new(stream, session_id).await?);
                }
                Err(e) => {
                    debug!("Failed to connect to {}: {}", addr, e);
                    TELEMETRY.record_backend_request_end(
                        &self.config.config_id,
                        addr,
                        now,
                        false,
                        &session_id,
                        Some(&e),
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(match last_error {
            Some(e) => e.into(),
            None => ProxyProtocolError::Other("Failed to connect to any server".to_string()),
        })
    }
}
