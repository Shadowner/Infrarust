use std::{net::SocketAddr, sync::Arc};

use infrarust_config::ServerConfig;
use tokio::net::TcpStream;
use tracing::{Instrument, debug, debug_span, instrument};
use uuid::Uuid;

use crate::{
    ProxyProtocolConfig, ServerConnection,
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
            return Err(ProxyProtocolError::Io(
                "No server addresses configured".into(),
            ));
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

    #[instrument(name = "fetch_status_directly", skip(self), fields(
        server_addr = %self.config.addresses.first().unwrap_or(&String::new()),
        domain = %req.domain
    ))]
    pub async fn fetch_status_directly(&self, req: &ServerRequest) -> ProtocolResult<Packet> {
        let use_proxy_protocol = self.config.send_proxy_protocol.unwrap_or(false);
        let start_time = std::time::Instant::now();

        debug!(
            "Connecting to server for domain: {} (proxy protocol: {})",
            req.domain, use_proxy_protocol
        );

        let connect_result = if use_proxy_protocol {
            self.dial_with_proxy_protocol(req.session_id, req.client_addr)
                .instrument(debug_span!("connect_with_proxy"))
                .await
        } else {
            self.dial(req.session_id)
                .instrument(debug_span!("connect_standard"))
                .await
        };

        match connect_result {
            Ok(mut conn) => {
                debug!("Connected to server after {:?}", start_time.elapsed());

                let fetch_start = std::time::Instant::now();
                match self.fetch_status_from_connection(&mut conn, req).await {
                    Ok(packet) => {
                        debug!("Status fetched in {:?}", fetch_start.elapsed());
                        Ok(packet)
                    }
                    Err(e) => {
                        debug!("Status fetch failed: {}", e);
                        Err(e)
                    }
                }
            }
            Err(e) => {
                debug!("Connection failed: {}", e);
                Err(e)
            }
        }
    }

    #[instrument(skip(self, conn), fields(
        domain = %req.domain,
        session_id = %req.session_id
    ))]
    async fn fetch_status_from_connection(
        &self,
        conn: &mut ServerConnection,
        req: &ServerRequest,
    ) -> ProtocolResult<Packet> {
        if let Err(e) = conn.write_packet(&req.read_packets[0].clone()).await {
            debug!("Failed to send handshake: {}", e);
            return Err(e);
        }

        if let Err(e) = conn.write_packet(&req.read_packets[1].clone()).await {
            debug!("Failed to send status request: {}", e);
            return Err(e);
        }

        let start = std::time::Instant::now();
        let result = conn.read_packet().await;
        let elapsed = start.elapsed();

        match &result {
            Ok(_) => debug!("Got status response in {:?}", elapsed),
            Err(e) => debug!(
                "Failed to read status response: {} (after {:?})",
                e, elapsed
            ),
        }

        result
    }
}
