//! `ClientOnly` proxy mode handler.
//!
//! Authenticates the client against Mojang (RSA + AES-128-CFB8 + session server),
//! then connects to the backend in offline mode. The client-side connection is
//! encrypted, the backend-side is plain.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use infrarust_protocol::packets::login::{
    CLoginDisconnect, CLoginSuccess, CSetCompression, Property, SLoginAcknowledged,
};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use infrarust_transport::BackendConnector;

use crate::auth::mojang::MojangAuth;
use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, LoginData, RoutingData};
use crate::player::PlayerSession;
use crate::registry::ConnectionRegistry;
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;
use crate::session::proxy_loop::{ProxyLoopOutcome, proxy_loop};

/// Handles connections in `ClientOnly` proxy mode.
///
/// Flow:
/// 1. Authenticate client via Mojang (RSA exchange + session server)
/// 2. Send `LoginSuccess` to client
/// 3. Connect to backend in offline mode
/// 4. Consume backend's login response (without forwarding)
/// 5. Run `proxy_loop` for bidirectional forwarding
pub struct ClientOnlyHandler {
    backend_connector: Arc<BackendConnector>,
    registry: Arc<PacketRegistry>,
    connection_registry: Arc<ConnectionRegistry>,
    auth: Arc<MojangAuth>,
    #[cfg(feature = "telemetry")]
    metrics: Option<Arc<crate::telemetry::ProxyMetrics>>,
}

impl ClientOnlyHandler {
    /// Creates a new `ClientOnly` handler.
    pub const fn new(
        backend_connector: Arc<BackendConnector>,
        registry: Arc<PacketRegistry>,
        connection_registry: Arc<ConnectionRegistry>,
        auth: Arc<MojangAuth>,
    ) -> Self {
        Self {
            backend_connector,
            registry,
            connection_registry,
            auth,
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

    /// Handles a `ClientOnly`-mode connection.
    ///
    /// # Errors
    /// Returns `CoreError` on authentication failure, backend connection
    /// failure, or I/O errors during the proxy session.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(name = "proxy.session", skip_all, fields(mode = "client_only"))]
    pub async fn handle(
        &self,
        mut ctx: ConnectionContext,
        shutdown: CancellationToken,
    ) -> Result<(), CoreError> {
        let routing = ctx.require_extension::<RoutingData>("RoutingData")?.clone();
        let handshake = ctx
            .require_extension::<HandshakeData>("HandshakeData")?
            .clone();
        let login_data = ctx.require_extension::<LoginData>("LoginData")?.clone();

        let server_config = &routing.server_config;
        let version = handshake.protocol_version;

        // ── Phase 1: Client Authentication ──

        let mut client = ClientBridge::new(ctx.take_stream(), ctx.buffered_data.split(), version);

        // Mojang auth: RSA exchange → session verification → enable encryption
        let game_profile = self
            .auth
            .authenticate(&mut client, &login_data.username, &self.registry)
            .await?;

        tracing::info!(
            username = %game_profile.name,
            uuid = %game_profile.id,
            "client authenticated"
        );

        // Send LoginSuccess to client with the Mojang profile
        self.send_login_success(&mut client, &game_profile, version)
            .await?;

        // ── Handle LoginAcknowledged (1.20.2+) ──

        if version.no_less_than(ProtocolVersion::V1_20_2) {
            // Wait for client to acknowledge login success
            let frame = client
                .read_frame()
                .await?
                .ok_or(CoreError::ConnectionClosed)?;

            let decoded = self.registry.decode_frame(
                &frame,
                ConnectionState::Login,
                Direction::Serverbound,
                version,
            )?;

            match decoded {
                DecodedPacket::Typed { packet, .. }
                    if packet
                        .as_any()
                        .downcast_ref::<SLoginAcknowledged>()
                        .is_some() =>
                {
                    client.set_state(ConnectionState::Config);
                    tracing::debug!("client LoginAcknowledged → Config");
                }
                _ => {
                    return Err(CoreError::Auth(
                        "expected LoginAcknowledged from client".to_string(),
                    ));
                }
            }
        } else {
            client.set_state(ConnectionState::Play);
        }

        // ── Phase 2: Backend Login (Offline Mode) ──

        let backend_conn = match self
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
                client.disconnect(msg, &self.registry).await.ok();
                return Ok(());
            }
        };

        let mut backend = BackendBridge::new(backend_conn.into_stream());

        // Send handshake + login start with offline UUID
        backend
            .send_initial_packets_offline(
                &handshake,
                server_config,
                &game_profile.name,
                &self.registry,
            )
            .await?;

        // Consume backend's login response (SetCompression + LoginSuccess)
        // without forwarding to client (client already got ours)
        self.consume_backend_login(&mut client, &mut backend, version)
            .await?;

        // For 1.20.2+: send LoginAcknowledged to backend to transition it to Config
        if version.no_less_than(ProtocolVersion::V1_20_2) {
            let ack = SLoginAcknowledged;
            backend.send_packet(&ack, &self.registry).await?;
            backend.set_state(ConnectionState::Config);
            tracing::debug!("backend LoginAcknowledged → Config");
        }

        // ── Phase 3: Session ──

        let session_token = CancellationToken::new();
        let player_uuid = game_profile.uuid().unwrap_or_else(|_| uuid::Uuid::new_v4());
        let (cmd_tx, cmd_rx) = PlayerSession::channel();

        let api_profile = infrarust_api::types::GameProfile {
            uuid: player_uuid,
            username: game_profile.name.clone(),
            properties: game_profile.properties.iter().map(|p| {
                infrarust_api::types::ProfileProperty {
                    name: p.name.clone(),
                    value: p.value.clone(),
                    signature: p.signature.clone(),
                }
            }).collect(),
        };

        let player_session = Arc::new(PlayerSession::new(
            infrarust_api::types::PlayerId::new(player_uuid.as_u128() as u64),
            api_profile,
            infrarust_api::types::ProtocolVersion::new(version.0),
            ctx.peer_addr,
            Some(infrarust_api::types::ServerId::new(routing.config_id.clone())),
            true, // active: ClientOnly supports packet injection
            cmd_tx,
            session_token.clone(),
        ));

        let session_id = self.connection_registry.register(player_session);

        tracing::info!(
            session = %session_id,
            server = %routing.config_id,
            username = %game_profile.name,
            mode = "client_only",
            "session started"
        );

        // Record metrics
        #[cfg(feature = "telemetry")]
        if let Some(ref metrics) = self.metrics {
            metrics.record_connection_start(&routing.config_id, "client_only");
            metrics.record_player_join(&routing.config_id);
        }

        // Combine shutdown tokens
        let combined_shutdown = CancellationToken::new();
        let combined = combined_shutdown.clone();
        let global = shutdown.clone();
        let session = session_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                biased;
                () = global.cancelled() => combined.cancel(),
                () = session.cancelled() => combined.cancel(),
            }
        });

        // Proxy loop (Config → Play for 1.20.2+, or Play for older)
        let outcome =
            proxy_loop(&mut client, &mut backend, &self.registry, combined_shutdown, cmd_rx).await;

        // Cleanup
        let _ = self.connection_registry.unregister(&session_id);

        // Record end metrics
        #[cfg(feature = "telemetry")]
        if let Some(ref metrics) = self.metrics {
            let duration_secs = ctx.connected_at.elapsed().as_secs_f64();
            metrics.record_connection_end(duration_secs, &routing.config_id, "client_only");
            metrics.record_player_leave(&routing.config_id);
        }

        match &outcome {
            ProxyLoopOutcome::ClientDisconnected => {
                tracing::info!(session = %session_id, "client disconnected");
            }
            ProxyLoopOutcome::BackendDisconnected { reason } => {
                tracing::info!(session = %session_id, ?reason, "backend disconnected");
            }
            ProxyLoopOutcome::Shutdown => {
                tracing::debug!(session = %session_id, "shutdown");
            }
            ProxyLoopOutcome::Error(e) => {
                tracing::warn!(session = %session_id, error = %e, "session error");
            }
        }

        Ok(())
    }

    /// Sends a `LoginSuccess` packet to the client with the Mojang game profile.
    async fn send_login_success(
        &self,
        client: &mut ClientBridge,
        profile: &crate::auth::game_profile::GameProfile,
        version: ProtocolVersion,
    ) -> Result<(), CoreError> {
        let uuid = profile.uuid()?;

        let properties: Vec<Property> = profile
            .properties
            .iter()
            .map(|p| Property {
                name: p.name.clone(),
                value: p.value.clone(),
                signature: p.signature.clone(),
            })
            .collect();

        let login_success = CLoginSuccess {
            uuid,
            username: profile.name.clone(),
            properties,
            strict_error_handling: version.no_less_than(ProtocolVersion::V1_20_5)
                && version.no_greater_than(ProtocolVersion::V1_21),
        };

        client.send_packet(&login_success, &self.registry).await?;
        tracing::debug!("sent LoginSuccess to client");

        Ok(())
    }

    /// Consumes the backend's login response without forwarding to the client.
    ///
    /// The client already received our own `LoginSuccess`. We read the backend's
    /// `SetCompression` (activate compression on backend only) and `LoginSuccess`
    /// (consume without forwarding), then the backend login is complete.
    async fn consume_backend_login(
        &self,
        client: &mut ClientBridge,
        backend: &mut BackendBridge,
        version: ProtocolVersion,
    ) -> Result<(), CoreError> {
        loop {
            let frame = backend
                .read_frame()
                .await?
                .ok_or(CoreError::ConnectionClosed)?;

            let decoded = self.registry.decode_frame(
                &frame,
                ConnectionState::Login,
                Direction::Clientbound,
                version,
            )?;

            match decoded {
                DecodedPacket::Typed { packet, .. } => {
                    if let Some(set_comp) = packet.as_any().downcast_ref::<CSetCompression>() {
                        // Activate compression on backend side only
                        backend.set_compression(set_comp.threshold.0);
                        tracing::debug!(
                            threshold = set_comp.threshold.0,
                            "backend compression activated"
                        );
                        continue;
                    }

                    if packet.as_any().downcast_ref::<CLoginSuccess>().is_some() {
                        // Backend login complete — consume without forwarding
                        if version.less_than(ProtocolVersion::V1_20_2) {
                            backend.set_state(ConnectionState::Play);
                        }
                        // For 1.20.2+, transition happens after we send LoginAcknowledged
                        tracing::debug!("consumed backend LoginSuccess");
                        break;
                    }

                    if let Some(disconnect) = packet.as_any().downcast_ref::<CLoginDisconnect>() {
                        client
                            .disconnect("Backend refused connection", &self.registry)
                            .await?;
                        return Err(CoreError::Rejected(format!(
                            "backend refused login: {}",
                            disconnect.reason
                        )));
                    }
                }
                DecodedPacket::Opaque { id, .. } => {
                    tracing::debug!(id, "ignoring opaque packet during backend login");
                }
            }
        }

        Ok(())
    }
}
