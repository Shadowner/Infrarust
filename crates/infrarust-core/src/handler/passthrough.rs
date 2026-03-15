use std::sync::Arc;

use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use infrarust_config::DomainRewrite;
use infrarust_protocol::io::PacketEncoder;
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::{Packet, VarInt};
use infrarust_transport::{BackendConnector, select_forwarder};

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, LoginData, RoutingData};
use crate::registry::{ConnectionRegistry, SessionEntry};

/// Handles passthrough proxy connections.
///
/// Connects to the backend, forwards initial packets (handshake + login start),
/// registers the session, and starts bidirectional forwarding.
pub struct PassthroughHandler {
    backend_connector: Arc<BackendConnector>,
    registry: Arc<ConnectionRegistry>,
}

impl PassthroughHandler {
    /// Creates a new passthrough handler.
    pub const fn new(
        backend_connector: Arc<BackendConnector>,
        registry: Arc<ConnectionRegistry>,
    ) -> Self {
        Self {
            backend_connector,
            registry,
        }
    }

    /// Handles a login connection by forwarding to the backend.
    pub async fn handle(
        &self,
        mut ctx: ConnectionContext,
        shutdown: CancellationToken,
    ) -> Result<(), CoreError> {
        let routing = ctx.require_extension::<RoutingData>("RoutingData")?.clone();
        let handshake = ctx
            .require_extension::<HandshakeData>("HandshakeData")?
            .clone();
        let login_data = ctx.extensions.get::<LoginData>().cloned();

        let server_config = &routing.server_config;

        // Connect to backend
        let backend = self
            .backend_connector
            .connect(
                &routing.config_id,
                &server_config.addresses,
                server_config.timeouts.as_ref().map(|t| t.connect),
                server_config.send_proxy_protocol,
                &ctx.connection_info(),
            )
            .await?;

        // Forward initial packets to backend
        let mut backend = backend;
        self.forward_initial_packets(backend.stream_mut(), &handshake, server_config)
            .await?;

        // Register session
        let session_token = CancellationToken::new();
        let session_id = Uuid::new_v4();
        self.registry.register(SessionEntry {
            session_id,
            username: login_data.as_ref().map(|d| d.username.clone()),
            player_uuid: login_data.as_ref().and_then(|d| d.player_uuid),
            client_ip: ctx.client_ip,
            server_id: routing.config_id.clone(),
            proxy_mode: server_config.proxy_mode,
            connected_at: ctx.connected_at,
            shutdown_token: session_token.clone(),
        });

        tracing::info!(
            session = %session_id,
            server = %routing.config_id,
            username = ?login_data.as_ref().map(|d| &d.username),
            "session started"
        );

        // Bidirectional forward
        let client_stream = ctx.take_stream();
        let backend_stream = backend.into_stream();
        let forwarder = select_forwarder(server_config.proxy_mode);

        // Combine session and global shutdown tokens
        let combined_shutdown = CancellationToken::new();
        let combined = combined_shutdown.clone();
        let global = shutdown.clone();
        let session = session_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = global.cancelled() => combined.cancel(),
                () = session.cancelled() => combined.cancel(),
            }
        });

        let result = forwarder
            .forward(client_stream, backend_stream, combined_shutdown)
            .await;

        // Cleanup
        self.registry.unregister(&session_id);

        tracing::info!(
            session = %session_id,
            c2b = result.client_to_backend,
            b2c = result.backend_to_client,
            reason = ?result.reason,
            "session ended"
        );

        Ok(())
    }

    /// Forwards the initial handshake and login packets to the backend.
    ///
    /// Applies domain rewrite if configured.
    async fn forward_initial_packets(
        &self,
        backend: &mut tokio::net::TcpStream,
        handshake: &HandshakeData,
        server_config: &infrarust_config::ServerConfig,
    ) -> Result<(), CoreError> {
        match &server_config.domain_rewrite {
            DomainRewrite::None => {
                // Forward raw packets as-is
                for raw in &handshake.raw_packets {
                    backend.write_all(raw).await?;
                }
            }
            DomainRewrite::Explicit(new_domain) => {
                // Re-encode handshake with new domain, then forward rest as-is
                self.forward_with_rewritten_handshake(backend, handshake, new_domain)
                    .await?;
            }
            DomainRewrite::FromBackend => {
                // Use backend address as domain
                if let Some(addr) = server_config.addresses.first() {
                    self.forward_with_rewritten_handshake(backend, handshake, &addr.host)
                        .await?;
                } else {
                    // Fallback: forward as-is
                    for raw in &handshake.raw_packets {
                        backend.write_all(raw).await?;
                    }
                }
            }
            _ => {
                // Unknown variant (non-exhaustive future additions): forward as-is
                for raw in &handshake.raw_packets {
                    backend.write_all(raw).await?;
                }
            }
        }

        backend.flush().await?;
        Ok(())
    }

    /// Re-encodes the handshake packet with a new domain and forwards all packets.
    #[allow(clippy::unused_self)] // Method for API consistency
    async fn forward_with_rewritten_handshake(
        &self,
        backend: &mut tokio::net::TcpStream,
        handshake: &HandshakeData,
        new_domain: &str,
    ) -> Result<(), CoreError> {
        // Build a modified SHandshake packet
        let next_state = match handshake.intent {
            crate::pipeline::types::ConnectionIntent::Status => {
                infrarust_protocol::ConnectionState::Status
            }
            crate::pipeline::types::ConnectionIntent::Login
            | crate::pipeline::types::ConnectionIntent::Transfer => {
                infrarust_protocol::ConnectionState::Login
            }
        };

        let modified = SHandshake {
            protocol_version: VarInt(handshake.protocol_version.0),
            server_address: new_domain.to_string(),
            server_port: handshake.port,
            next_state,
        };

        // Encode the modified handshake
        let mut payload = Vec::new();
        modified.encode(&mut payload, handshake.protocol_version)?;

        let mut encoder = PacketEncoder::new();
        encoder.append_raw(0x00, &payload)?; // Handshake is always 0x00
        let bytes = encoder.take();
        backend.write_all(&bytes).await?;

        // Forward remaining packets (login start etc.) as-is
        for raw in handshake.raw_packets.iter().skip(1) {
            backend.write_all(raw).await?;
        }

        Ok(())
    }
}
