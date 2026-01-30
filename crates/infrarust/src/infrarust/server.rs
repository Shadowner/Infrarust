use std::{io, sync::Arc};

use infrarust_config::models::logging::LogType;
use infrarust_protocol::minecraft::java::handshake::ServerBoundHandshake;
use infrarust_protocol::version::Version;
use tokio::net::TcpListener;
use tracing::{Instrument, Span, debug, debug_span, error, info, instrument, warn};
use uuid::Uuid;

use crate::{Connection, Infrarust, core::error::SendError, server::ServerRequest};

impl Infrarust {
    pub async fn run(self: Arc<Self>) -> Result<(), SendError> {
        debug!(
            log_type = LogType::Supervisor.as_str(),
            "Starting Infrarust server"
        );
        let bind_addr = self.shared.config().bind.clone().unwrap_or_default();

        // Create listener
        let listener = match TcpListener::bind(&bind_addr).await {
            Ok(l) => l,
            Err(e) => {
                error!(
                    log_type = LogType::TcpConnection.as_str(),
                    "Failed to bind to {}: {}", bind_addr, e
                );
                self.shared
                    .shutdown_controller()
                    .trigger_shutdown(&format!("Failed to bind: {}", e))
                    .await;
                return Err(SendError::new(e));
            }
        };

        info!(
            log_type = LogType::TcpConnection.as_str(),
            "Listening on {}", bind_addr
        );

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

        info!(
            log_type = LogType::Supervisor.as_str(),
            "Server stopped accepting new connections"
        );
        Ok(())
    }

    #[instrument(name = "connection_flow", skip(client), fields(log_type = LogType::TcpConnection.as_str()))]
    pub(crate) async fn handle_connection(&self, mut client: Connection) -> io::Result<()> {
        let peer_addr = client.peer_addr().await?;
        Span::current().record("peer_addr", format!("{}", peer_addr));

        debug!(
            log_type = LogType::TcpConnection.as_str(),
            "Starting to process new connection from {}", peer_addr
        );

        let handshake_timeout_secs = self.shared.config().handshake_timeout_secs.unwrap_or(10);

        // Peek the first byte to detect legacy protocol before reading any packets
        let first_byte = match tokio::time::timeout(
            tokio::time::Duration::from_secs(handshake_timeout_secs),
            client.peek_first_byte(),
        )
        .await
        {
            Ok(Ok(byte)) => byte,
            Ok(Err(e)) => {
                debug!(
                    log_type = LogType::TcpConnection.as_str(),
                    "Failed to peek first byte: {}", e
                );
                let _ = client.close().await;
                return Err(e);
            }
            Err(_) => {
                debug!(
                    log_type = LogType::TcpConnection.as_str(),
                    "Timeout waiting for first byte after {}s", handshake_timeout_secs
                );
                let _ = client.close().await;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "First byte timeout",
                ));
            }
        };

        let client_addr = peer_addr;
        let session_id = client.session_id;

        match first_byte {
            0xFE => {
                // Legacy server list ping (Beta 1.8 through 1.6)
                debug!(
                    log_type = LogType::PacketProcessing.as_str(),
                    "Detected legacy ping (0xFE) from {}", client_addr
                );
                crate::server::legacy_handler::handle_legacy_ping(
                    &mut client,
                    &self.gateway,
                    session_id,
                    client_addr,
                )
                .await
            }
            0x02 => {
                // Legacy login handshake (Beta 1.8 through 1.6)
                debug!(
                    log_type = LogType::PacketProcessing.as_str(),
                    "Detected legacy login handshake (0x02) from {}", client_addr
                );
                crate::server::legacy_handler::handle_legacy_login(
                    client,
                    &self.gateway,
                    session_id,
                    client_addr,
                )
                .await
            }
            _ => {
                // Modern protocol (1.7+) — VarInt-prefixed packets
                self.handle_modern_connection(client, handshake_timeout_secs)
                    .await
            }
        }
    }

    /// Handle a modern (1.7+) Minecraft connection with VarInt-framed packets.
    async fn handle_modern_connection(
        &self,
        mut client: Connection,
        handshake_timeout_secs: u64,
    ) -> io::Result<()> {
        debug!(
            log_type = LogType::PacketProcessing.as_str(),
            "Reading handshake packet (with {}s timeout)", handshake_timeout_secs
        );
        let handshake_packet = match tokio::time::timeout(
            tokio::time::Duration::from_secs(handshake_timeout_secs),
            client.read_packet(),
        )
        .await
        {
            Ok(Ok(packet)) => {
                debug!(
                    log_type = LogType::PacketProcessing.as_str(),
                    "Successfully read handshake packet"
                );
                packet
            }
            Ok(Err(e)) => {
                debug!(
                    log_type = LogType::PacketProcessing.as_str(),
                    "Failed to read handshake packet: {}", e
                );
                if let Err(close_err) = client.close().await {
                    warn!(
                        log_type = LogType::TcpConnection.as_str(),
                        "Error closing client connection: {}", close_err
                    );
                }
                return Err(e.into());
            }
            Err(_) => {
                debug!(
                    log_type = LogType::PacketProcessing.as_str(),
                    "Timeout reading handshake packet after {}s", handshake_timeout_secs
                );
                if let Err(close_err) = client.close().await {
                    warn!(
                        log_type = LogType::TcpConnection.as_str(),
                        "Error closing client connection: {}", close_err
                    );
                }
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Handshake packet timeout",
                ));
            }
        };

        debug!(
            log_type = LogType::PacketProcessing.as_str(),
            "Parsing handshake packet"
        );
        let handshake = match ServerBoundHandshake::from_packet(&handshake_packet) {
            Ok(handshake) => {
                debug!(
                    log_type = LogType::PacketProcessing.as_str(),
                    "Successfully parsed handshake: {:?}", handshake
                );
                handshake
            }
            Err(e) => {
                debug!(
                    log_type = LogType::PacketProcessing.as_str(),
                    "Failed to parse handshake packet: {}", e
                );
                if let Err(close_err) = client.close().await {
                    warn!(
                        log_type = LogType::TcpConnection.as_str(),
                        "Error closing client connection: {}", close_err
                    );
                }
                return Err(io::Error::new(io::ErrorKind::InvalidData, e));
            }
        };

        let domain: Arc<str> = handshake.parse_server_address().into();
        debug!(domain = %domain, log_type = LogType::PacketProcessing.as_str(), "Processing connection for domain");

        debug!(
            log_type = LogType::PacketProcessing.as_str(),
            "Reading second packet (with {}s timeout)", handshake_timeout_secs
        );
        let second_packet = match tokio::time::timeout(
            tokio::time::Duration::from_secs(handshake_timeout_secs),
            client.read_packet(),
        )
        .await
        {
            Ok(Ok(packet)) => {
                debug!(
                    log_type = LogType::PacketProcessing.as_str(),
                    "Successfully read second packet"
                );
                packet
            }
            Ok(Err(e)) => {
                debug!(
                    log_type = LogType::PacketProcessing.as_str(),
                    "Failed to read second packet: {}", e
                );
                if let Err(close_err) = client.close().await {
                    warn!(
                        log_type = LogType::TcpConnection.as_str(),
                        "Error closing client connection: {}", close_err
                    );
                }
                return Err(e.into());
            }
            Err(_) => {
                debug!(
                    log_type = LogType::PacketProcessing.as_str(),
                    "Timeout reading second packet after {}s", handshake_timeout_secs
                );
                if let Err(close_err) = client.close().await {
                    warn!(
                        log_type = LogType::TcpConnection.as_str(),
                        "Error closing client connection: {}", close_err
                    );
                }
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Second packet timeout",
                ));
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
            debug!(
                log_type = LogType::ServerManager.as_str(),
                "Processing client in separate task"
            );

            if let Some(original_addr) = &original_client_addr {
                debug!(
                    log_type = LogType::ProxyProtocol.as_str(),
                    "Using original client address from proxy protocol: {}", original_addr
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
                        read_packets: Arc::new([handshake_packet, second_packet]),
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
            let config_service = self_guard.shared.configuration_service_arc();
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

    pub fn get_shared(&self) -> Arc<crate::core::shared_component::SharedComponent> {
        self.shared.clone()
    }
}
