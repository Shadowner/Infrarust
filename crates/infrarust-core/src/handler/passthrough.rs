use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use infrarust_config::DomainRewrite;
use infrarust_protocol::io::PacketEncoder;
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::login::CLoginDisconnect;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use infrarust_protocol::{Packet, VarInt};
use infrarust_transport::{BackendConnector, select_forwarder};

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, LoginData, RoutingData};
use crate::player::PlayerSession;
use crate::services::ProxyServices;

/// Handles passthrough proxy connections.
///
/// Connects to the backend, forwards initial packets (handshake + login start),
/// registers the session, and starts bidirectional forwarding.
pub struct PassthroughHandler {
    backend_connector: Arc<BackendConnector>,
    services: ProxyServices,
    #[cfg(feature = "telemetry")]
    metrics: Option<Arc<crate::telemetry::ProxyMetrics>>,
}

impl PassthroughHandler {
    /// Creates a new passthrough handler.
    pub fn new(
        backend_connector: Arc<BackendConnector>,
        services: ProxyServices,
    ) -> Self {
        Self {
            backend_connector,
            services,
            #[cfg(feature = "telemetry")]
            metrics: None,
        }
    }

    /// Sets the metrics collector (telemetry feature only).
    #[cfg(feature = "telemetry")]
    pub fn with_metrics(mut self, metrics: Arc<crate::telemetry::ProxyMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Handles a login connection by forwarding to the backend.
    ///
    /// # Errors
    /// Returns `CoreError` on backend connection failure or I/O errors.
    #[tracing::instrument(name = "proxy.session", skip_all, fields(mode = "passthrough"))]
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

        // Build player_id and api_profile early for events
        let player_uuid = login_data
            .as_ref()
            .and_then(|d| d.player_uuid)
            .unwrap_or_else(uuid::Uuid::new_v4);
        let username = login_data
            .as_ref()
            .map(|d| d.username.clone())
            .unwrap_or_default();
        let player_id = crate::player::next_player_id();
        let api_profile = infrarust_api::types::GameProfile {
            uuid: player_uuid,
            username: username.clone(),
            properties: vec![],
        };

        // ── ServerPreConnectEvent ──
        let initial_server = infrarust_api::types::ServerId::new(routing.config_id.clone());
        let pre_connect = infrarust_api::events::connection::ServerPreConnectEvent::new(
            player_id, api_profile.clone(), initial_server,
        );
        let pre_connect = self.services.event_bus.fire(pre_connect).await;
        match pre_connect.result() {
            infrarust_api::events::connection::ServerPreConnectResult::Allowed => {}
            infrarust_api::events::connection::ServerPreConnectResult::Denied { reason } => {
                let json_reason = reason.to_json();
                let packet = CLoginDisconnect { reason: json_reason };
                let packet_id = self.services.packet_registry
                    .get_packet_id::<CLoginDisconnect>(
                        ConnectionState::Login,
                        Direction::Clientbound,
                        handshake.protocol_version,
                    )
                    .unwrap_or(0x00);
                let mut payload = Vec::new();
                packet.encode(&mut payload, handshake.protocol_version)?;
                let mut encoder = PacketEncoder::new();
                encoder.append_raw(packet_id, &payload)?;
                let bytes = encoder.take();
                ctx.stream_mut().write_all(&bytes).await?;
                ctx.stream_mut().flush().await?;
                return Ok(());
            }
            _ => {} // ConnectTo, SendToLimbo, VirtualBackend — Phase 4
        }

        // Connect to backend
        let backend = match self
            .backend_connector
            .connect(
                &routing.config_id,
                &server_config.addresses,
                server_config.timeouts.as_ref().map(|t| t.connect),
                server_config.send_proxy_protocol,
                &ctx.connection_info(),
            )
            .await
        {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    server = %routing.config_id,
                    error = %e,
                    "backend unreachable, sending disconnect to client"
                );
                let msg = server_config.effective_disconnect_message();
                self.send_kick_raw(ctx.stream_mut(), msg, handshake.protocol_version)
                    .await
                    .ok();
                return Ok(());
            }
        };

        // Forward initial packets to backend
        let mut backend = backend;
        self.forward_initial_packets(backend.stream_mut(), &handshake, server_config)
            .await?;

        // ── ServerConnectedEvent (fire-and-forget) ──
        self.services.event_bus.fire_and_forget_arc(infrarust_api::events::connection::ServerConnectedEvent {
            player_id,
            server: infrarust_api::types::ServerId::new(routing.config_id.clone()),
        });

        // Register session
        let session_token = shutdown.child_token();
        let (cmd_tx, _cmd_rx) = PlayerSession::channel();

        let player_session = Arc::new(PlayerSession::new(
            player_id,
            api_profile,
            infrarust_api::types::ProtocolVersion::new(handshake.protocol_version.0),
            ctx.peer_addr,
            Some(infrarust_api::types::ServerId::new(routing.config_id.clone())),
            false, // active: Passthrough doesn't support packet injection
            cmd_tx,
            session_token.clone(),
        ));

        let session_id = self.services.connection_registry.register(player_session);

        tracing::info!(
            session = %session_id,
            server = %routing.config_id,
            username = ?login_data.as_ref().map(|d| &d.username),
            "session started"
        );

        // Record metrics
        #[cfg(feature = "telemetry")]
        if let Some(ref metrics) = self.metrics {
            metrics.record_connection_start(&routing.config_id, "passthrough");
            metrics.record_player_join(&routing.config_id);
        }

        // Bidirectional forward
        let client_stream = ctx.take_stream();
        let backend_stream = backend.into_stream();
        let forwarder = select_forwarder(server_config.proxy_mode);

        let result = forwarder
            .forward(client_stream, backend_stream, session_token.clone())
            .await;

        // ── DisconnectEvent (always) ──
        let disconnect = infrarust_api::events::lifecycle::DisconnectEvent {
            player_id,
            username,
            last_server: Some(infrarust_api::types::ServerId::new(routing.config_id.clone())),
        };
        let _ = self.services.event_bus.fire(disconnect).await;

        // Cleanup
        let _ = self.services.connection_registry.unregister(&session_id);

        // Record end metrics
        #[cfg(feature = "telemetry")]
        if let Some(ref metrics) = self.metrics {
            let duration_secs = ctx.connected_at.elapsed().as_secs_f64();
            metrics.record_connection_end(duration_secs, &routing.config_id, "passthrough");
            metrics.record_player_leave(&routing.config_id);
        }

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

    /// Sends a login disconnect packet directly to the client stream.
    async fn send_kick_raw(
        &self,
        stream: &mut tokio::net::TcpStream,
        reason: &str,
        version: ProtocolVersion,
    ) -> Result<(), CoreError> {
        let json_reason = serde_json::json!({"text": reason}).to_string();
        let packet = CLoginDisconnect {
            reason: json_reason,
        };

        let packet_id = self
            .services
            .packet_registry
            .get_packet_id::<CLoginDisconnect>(
                ConnectionState::Login,
                Direction::Clientbound,
                version,
            )
            .unwrap_or(0x00);

        let mut payload = Vec::new();
        packet.encode(&mut payload, version)?;

        let mut encoder = PacketEncoder::new();
        encoder.append_raw(packet_id, &payload)?;
        let bytes = encoder.take();

        stream.write_all(&bytes).await?;
        stream.flush().await?;
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
