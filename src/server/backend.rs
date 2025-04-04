use std::{net::SocketAddr, sync::Arc};

use tokio::net::TcpStream;
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::{
    ProxyProtocolConfig, ServerConnection,
    core::config::ServerConfig,
    network::{
        packet::Packet,
        proxy_protocol::{ProtocolResult, errors::ProxyProtocolError},
    },
    write_proxy_protocol_header,
};

#[cfg(feature = "telemetry")]
use crate::telemetry::TELEMETRY;

use super::ServerRequest;

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

    #[instrument(skip(self), fields(
        addresses = ?self.config.addresses,
        session_id = %session_id,
        config_id = %self.config.config_id
    ))]
    pub async fn dial(&self, session_id: Uuid) -> ProtocolResult<ServerConnection> {
        let mut last_error = None;
        debug!("Dialing server with addresses: {:?}", self.config.addresses);

        if self.config.addresses.is_empty() {
            debug!("No addresses to connect to!");
            return Err(ProxyProtocolError::Other(
                "No server addresses configured".to_string(),
            ));
        }

        for (i, addr) in self.config.addresses.iter().enumerate() {
            debug!("Attempt {} - Connecting to {}", i + 1, addr);
            let now = std::time::Instant::now();

            #[cfg(feature = "telemetry")]
            TELEMETRY.record_backend_request_start(&self.config.config_id, addr, &session_id);

            match tokio::time::timeout(std::time::Duration::from_secs(5), TcpStream::connect(addr))
                .await
            {
                Ok(Ok(stream)) => {
                    debug!(
                        "Connected to {} successfully after {:?}",
                        addr,
                        now.elapsed()
                    );
                    match stream.set_nodelay(true) {
                        Ok(_) => debug!("Set TCP_NODELAY successfully"),
                        Err(e) => debug!("Failed to set TCP_NODELAY: {}", e),
                    }

                    #[cfg(feature = "telemetry")]
                    TELEMETRY.record_backend_request_end(
                        &self.config.config_id,
                        addr,
                        now,
                        true,
                        &session_id,
                        None,
                    );

                    debug!("Creating server connection");
                    let conn_result = ServerConnection::new(stream, session_id).await;
                    match &conn_result {
                        Ok(_) => debug!("Server connection created successfully"),
                        Err(e) => debug!("Failed to create server connection: {}", e),
                    }
                    return conn_result.map_err(|e| e.into());
                }
                Ok(Err(e)) => {
                    debug!(
                        "Failed to connect to {} after {:?}: {}",
                        addr,
                        now.elapsed(),
                        e
                    );

                    #[cfg(feature = "telemetry")]
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
                Err(_) => {
                    debug!("Connection to {} timed out after 5 seconds", addr);
                    let e = std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("Connection to {} timed out", addr),
                    );

                    #[cfg(feature = "telemetry")]
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

        debug!("Failed to connect to any server addresses");
        Err(match last_error {
            Some(e) => {
                debug!("Last error: {}", e);
                e.into()
            }
            None => {
                debug!("No error details available");
                ProxyProtocolError::Other("Failed to connect to any server".to_string())
            }
        })
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
            #[cfg(feature = "telemetry")]
            let now = std::time::Instant::now();
            #[cfg(feature = "telemetry")]
            TELEMETRY.record_backend_request_start(&self.config.config_id, addr, &session_id);

            match TcpStream::connect(addr).await {
                Ok(mut stream) => {
                    debug!("Connected to {}", addr);
                    stream.set_nodelay(true)?;

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

                                #[cfg(feature = "telemetry")]
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

                    #[cfg(feature = "telemetry")]
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

                    #[cfg(feature = "telemetry")]
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

    pub async fn fetch_status_directly(&self, req: &ServerRequest) -> ProtocolResult<Packet> {
        debug!("Directly fetching status for {}", req.domain);

        let use_proxy_protocol = self.config.send_proxy_protocol.unwrap_or(false);
        let conn = if use_proxy_protocol {
            debug!("Using proxy protocol for direct status connection");
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                self.dial_with_proxy_protocol(req.session_id, req.client_addr),
            )
            .await
            {
                Ok(Ok(conn)) => conn,
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(ProxyProtocolError::Other("Connection timeout".to_string())),
            }
        } else {
            debug!("Using standard connection for direct status");
            match tokio::time::timeout(std::time::Duration::from_secs(5), self.dial(req.session_id))
                .await
            {
                Ok(Ok(conn)) => conn,
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(ProxyProtocolError::Other("Connection timeout".to_string())),
            }
        };

        let mut conn = conn;
        debug!("Sending handshake packet for direct status");
        if let Err(e) = conn.write_packet(&req.read_packets[0].clone()).await {
            debug!("Failed to send handshake packet: {}", e);
            return Err(e);
        }

        debug!("Sending status request packet for direct status");
        if let Err(e) = conn.write_packet(&req.read_packets[1].clone()).await {
            debug!("Failed to send status request packet: {}", e);
            return Err(e);
        }

        debug!("Waiting for direct status response");
        match tokio::time::timeout(std::time::Duration::from_secs(5), conn.read_packet()).await {
            Ok(Ok(packet)) => {
                debug!("Received direct status response");

                if let Some(motd) = &self.config.motd {
                    debug!("Using custom MOTD for direct status");
                    return crate::server::motd::generate_motd(motd, false);
                }

                Ok(packet)
            }
            Ok(Err(e)) => {
                debug!("Error reading direct status response: {}", e);
                Err(e)
            }
            Err(_) => {
                debug!("Timeout reading direct status response");
                Err(ProxyProtocolError::Other(
                    "Status response timeout".to_string(),
                ))
            }
        }
    }
}
