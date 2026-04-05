use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use infrarust_config::DomainRewrite;
use infrarust_protocol::Packet;
use infrarust_protocol::io::PacketEncoder;
use infrarust_protocol::version::ProtocolVersion;
use infrarust_transport::{BackendConnector, select_forwarder};

use crate::error::CoreError;
use crate::forwarding::{ForwardingData, ForwardingHandler, build_handshake_for_backend};
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
    pub fn new(backend_connector: Arc<BackendConnector>, services: ProxyServices) -> Self {
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

        let initial_server = infrarust_api::types::ServerId::new(routing.config_id.clone());
        let pre_connect = infrarust_api::events::connection::ServerPreConnectEvent::new(
            player_id,
            api_profile.clone(),
            initial_server,
        );
        let pre_connect = self.services.event_bus.fire(pre_connect).await;
        match pre_connect.result() {
            infrarust_api::events::connection::ServerPreConnectResult::Allowed => {}
            infrarust_api::events::connection::ServerPreConnectResult::Denied { reason } => {
                super::helpers::send_login_disconnect(
                    ctx.stream_mut(),
                    &reason.to_json(),
                    handshake.protocol_version,
                    &self.services.packet_registry,
                )
                .await
                .ok();
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
        let fwd_data = ForwardingData {
            real_ip: ctx.client_ip,
            uuid: player_uuid,
            username: username.clone(),
            properties: vec![], // No properties in passthrough (no Mojang auth)
            protocol_version: handshake.protocol_version,
            chat_session: None,
        };
        self.forward_initial_packets(backend.stream_mut(), &handshake, server_config, &fwd_data)
            .await?;

        self.services.event_bus.fire_and_forget_arc(
            infrarust_api::events::connection::ServerConnectedEvent {
                player_id,
                server: infrarust_api::types::ServerId::new(routing.config_id.clone()),
            },
        );

        // Register session
        let session_token = shutdown.child_token();
        let (cmd_tx, _cmd_rx) = PlayerSession::channel();

        let player_session = Arc::new(PlayerSession::new(
            player_id,
            api_profile,
            infrarust_api::types::ProtocolVersion::new(handshake.protocol_version.0),
            ctx.peer_addr,
            Some(infrarust_api::types::ServerId::new(
                routing.config_id.clone(),
            )),
            false, // active: Passthrough doesn't support packet injection
            false, // online_mode: passthrough doesn't authenticate
            cmd_tx,
            session_token.clone(),
            crate::permissions::default_checker(),
        ));

        let session_id = self.services.connection_registry.register(player_session);

        tracing::info!(
            session = %session_id,
            server = %routing.config_id,
            username = ?login_data.as_ref().map(|d| &d.username),
            mode = "passthrough",
            "session started"
        );

        // Record metrics
        #[cfg(feature = "telemetry")]
        super::helpers::record_session_start(&self.metrics, &routing.config_id, "passthrough");

        // Bidirectional forward
        let client_stream = ctx.take_stream();
        let backend_stream = backend.into_stream();
        let forwarder = select_forwarder(server_config.proxy_mode);

        let result = forwarder
            .forward(client_stream, backend_stream, session_token.clone())
            .await;

        super::helpers::fire_disconnect_event(
            &self.services.event_bus,
            player_id,
            username,
            Some(infrarust_api::types::ServerId::new(
                routing.config_id.clone(),
            )),
        )
        .await;

        // Cleanup
        let _ = self.services.connection_registry.unregister(&session_id);

        // Record end metrics
        #[cfg(feature = "telemetry")]
        super::helpers::record_session_end(
            &self.metrics,
            ctx.connection_duration(),
            &routing.config_id,
            "passthrough",
        );

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
    /// Applies domain rewrite and forwarding data injection if configured.
    async fn forward_initial_packets(
        &self,
        backend: &mut tokio::net::TcpStream,
        handshake: &HandshakeData,
        server_config: &infrarust_config::ServerConfig,
        fwd_data: &ForwardingData,
    ) -> Result<(), CoreError> {
        let handler = self.services.resolve_forwarding_handler(server_config);

        if matches!(handler, ForwardingHandler::Velocity(_)) {
            tracing::warn!(
                "Velocity forwarding is configured for server '{}' in passthrough mode. \
                 Velocity requires packet parsing and cannot work with passthrough. \
                 Falling back to BungeeCord legacy forwarding.",
                server_config.effective_id()
            );
            let fallback =
                ForwardingHandler::Legacy(crate::forwarding::legacy::LegacyForwardingHandler);
            return self
                .forward_with_forwarding(backend, handshake, server_config, fwd_data, &fallback)
                .await;
        }

        if handler.modifies_handshake() {
            return self
                .forward_with_forwarding(backend, handshake, server_config, fwd_data, &handler)
                .await;
        }

        match &server_config.domain_rewrite {
            DomainRewrite::None => {
                for raw in &handshake.raw_packets {
                    backend.write_all(raw).await?;
                }
            }
            DomainRewrite::Explicit(new_domain) => {
                self.forward_with_rewritten_handshake(backend, handshake, new_domain)
                    .await?;
            }
            DomainRewrite::FromBackend => {
                if let Some(addr) = server_config.addresses.first() {
                    self.forward_with_rewritten_handshake(backend, handshake, &addr.host)
                        .await?;
                } else {
                    for raw in &handshake.raw_packets {
                        backend.write_all(raw).await?;
                    }
                }
            }
            _ => {
                for raw in &handshake.raw_packets {
                    backend.write_all(raw).await?;
                }
            }
        }

        backend.flush().await?;
        Ok(())
    }

    async fn forward_with_forwarding(
        &self,
        backend: &mut tokio::net::TcpStream,
        handshake: &HandshakeData,
        server_config: &infrarust_config::ServerConfig,
        fwd_data: &ForwardingData,
        handler: &ForwardingHandler,
    ) -> Result<(), CoreError> {
        let mut modified = build_handshake_for_backend(handshake, server_config);

        handler.apply_handshake(&mut modified, fwd_data);

        let mut payload = Vec::new();
        modified.encode(&mut payload, handshake.protocol_version)?;

        let mut encoder = PacketEncoder::new();
        encoder.append_raw(0x00, &payload)?;
        let bytes = encoder.take();
        backend.write_all(&bytes).await?;

        for raw in handshake.raw_packets.iter().skip(1) {
            backend.write_all(raw).await?;
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
        super::helpers::send_login_disconnect(
            stream,
            reason,
            version,
            &self.services.packet_registry,
        )
        .await
    }

    /// Re-encodes the handshake packet with a new domain and forwards all packets.
    #[allow(clippy::unused_self)] // Method for API consistency
    async fn forward_with_rewritten_handshake(
        &self,
        backend: &mut tokio::net::TcpStream,
        handshake: &HandshakeData,
        new_domain: &str,
    ) -> Result<(), CoreError> {
        let encoded =
            crate::util::domain_rewrite::encode_handshake_with_domain(handshake, new_domain)?;
        backend.write_all(&encoded).await?;

        // Forward remaining packets (login start etc.) as-is
        for raw in handshake.raw_packets.iter().skip(1) {
            backend.write_all(raw).await?;
        }

        Ok(())
    }
}
