//! `ClientOnly` proxy mode handler.
//!
//! Authenticates the client against Mojang (RSA + AES-128-CFB8 + session server),
//! then connects to the backend in offline mode. The client-side connection is
//! encrypted, the backend-side is plain.

use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use tokio_util::sync::CancellationToken;

use infrarust_protocol::packets::login::{
    CLoginDisconnect, CLoginSuccess, CSetCompression, Property, SLoginAcknowledged,
};
use infrarust_protocol::registry::DecodedPacket;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use infrarust_transport::BackendConnector;

use crate::auth::mojang::MojangAuth;
use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, LoginData, RoutingData};
use crate::player::PlayerSession;
use crate::services::ProxyServices;
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
    services: ProxyServices,
    auth: Arc<MojangAuth>,
    #[cfg(feature = "telemetry")]
    metrics: Option<Arc<crate::telemetry::ProxyMetrics>>,
}

impl ClientOnlyHandler {
    /// Creates a new `ClientOnly` handler.
    pub fn new(
        backend_connector: Arc<BackendConnector>,
        services: ProxyServices,
        auth: Arc<MojangAuth>,
    ) -> Self {
        Self {
            backend_connector,
            services,
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

        // ── PreLoginEvent ──
        let pre_login_profile = infrarust_api::types::GameProfile {
            uuid: uuid::Uuid::nil(),
            username: login_data.username.clone(),
            properties: vec![],
        };
        let pre_login = infrarust_api::events::lifecycle::PreLoginEvent::new(
            pre_login_profile,
            ctx.peer_addr,
            infrarust_api::types::ProtocolVersion::new(version.0),
            handshake.domain.clone(),
        );
        let pre_login = self.services.event_bus.fire(pre_login).await;
        if let infrarust_api::events::lifecycle::PreLoginResult::Denied { reason } = pre_login.result() {
            let reason_json = reason.to_json();
            client.disconnect(&reason_json, &self.services.packet_registry).await.ok();
            return Ok(());
        }

        // Mojang auth: RSA exchange → session verification → enable encryption
        let game_profile = self
            .auth
            .authenticate(&mut client, &login_data.username, &self.services.packet_registry)
            .await?;

        tracing::info!(
            username = %game_profile.name,
            uuid = %game_profile.id,
            "client authenticated"
        );

        // Build api_profile and player_id early for events
        let player_uuid = game_profile.uuid().unwrap_or_else(|_| uuid::Uuid::new_v4());
        let player_id = crate::player::next_player_id();
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

        // Send LoginSuccess to client with the Mojang profile
        self.send_login_success(&mut client, &game_profile, version)
            .await?;

        // ── PostLoginEvent (fire-and-forget) ──
        self.services.event_bus.fire_and_forget_arc(infrarust_api::events::lifecycle::PostLoginEvent {
            profile: api_profile.clone(),
            player_id,
            protocol_version: infrarust_api::types::ProtocolVersion::new(version.0),
        });

        // ── Handle LoginAcknowledged (1.20.2+) ──

        if version.no_less_than(ProtocolVersion::V1_20_2) {
            // Wait for client to acknowledge login success
            let frame = client
                .read_frame()
                .await?
                .ok_or(CoreError::ConnectionClosed)?;

            let decoded = self.services.packet_registry.decode_frame(
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

        // ── PlayerChooseInitialServerEvent ──
        let initial_server = infrarust_api::types::ServerId::new(routing.config_id.clone());
        let choose = infrarust_api::events::connection::PlayerChooseInitialServerEvent::new(
            player_id, api_profile.clone(), initial_server.clone(),
        );
        let choose = self.services.event_bus.fire(choose).await;
        let target_server_id = match choose.result() {
            infrarust_api::events::connection::PlayerChooseInitialServerResult::Allowed => initial_server,
            infrarust_api::events::connection::PlayerChooseInitialServerResult::Redirect(id) => id.clone(),
            _ => initial_server,
        };

        // ── ServerPreConnectEvent ──
        let pre_connect = infrarust_api::events::connection::ServerPreConnectEvent::new(
            player_id, api_profile.clone(), target_server_id.clone(),
        );
        let pre_connect = self.services.event_bus.fire(pre_connect).await;
        match pre_connect.result() {
            infrarust_api::events::connection::ServerPreConnectResult::Allowed => {}
            infrarust_api::events::connection::ServerPreConnectResult::Denied { reason } => {
                let reason_json = reason.to_json();
                client.disconnect(&reason_json, &self.services.packet_registry).await.ok();
                return Ok(());
            }
            _ => {} // ConnectTo, SendToLimbo, VirtualBackend — Phase 4
        }

        // ── Phase 2: Backend Login (Offline Mode) ──

        // TODO: Phase 4 — resolve target_server_id to addresses for backend connection
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
                client.disconnect(msg, &self.services.packet_registry).await.ok();
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
                &self.services.packet_registry,
            )
            .await?;

        // Consume backend's login response (SetCompression + LoginSuccess)
        // without forwarding to client (client already got ours)
        self.consume_backend_login(&mut client, &mut backend, version)
            .await?;

        // For 1.20.2+: send LoginAcknowledged to backend to transition it to Config
        if version.no_less_than(ProtocolVersion::V1_20_2) {
            let ack = SLoginAcknowledged;
            backend.send_packet(&ack, &self.services.packet_registry).await?;
            backend.set_state(ConnectionState::Config);
            tracing::debug!("backend LoginAcknowledged → Config");
        }

        // ── ServerConnectedEvent (fire-and-forget) ──
        self.services.event_bus.fire_and_forget_arc(infrarust_api::events::connection::ServerConnectedEvent {
            player_id,
            server: target_server_id.clone(),
        });

        // ── Phase 3: Session ──

        let session_token = shutdown.child_token();
        let (cmd_tx, cmd_rx) = PlayerSession::channel();

        let player_session = Arc::new(PlayerSession::new(
            player_id,
            api_profile.clone(),
            infrarust_api::types::ProtocolVersion::new(version.0),
            ctx.peer_addr,
            Some(infrarust_api::types::ServerId::new(routing.config_id.clone())),
            true, // active: ClientOnly supports packet injection
            cmd_tx,
            session_token.clone(),
        ));

        let session_id = self.services.connection_registry.register(player_session);

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

        // Build codec filter chains
        let (mut client_codec_chain, mut server_codec_chain) =
            crate::filter::codec_chain::build_codec_chains(
                &self.services.codec_filter_registry,
                infrarust_api::types::ProtocolVersion::new(handshake.protocol_version.0),
                player_id.as_u64(),
                ctx.peer_addr,
                Some(ctx.client_ip),
            );

        // Proxy loop (Config → Play for 1.20.2+, or Play for older)
        let outcome =
            proxy_loop(&mut client, &mut backend, &self.services.packet_registry, session_token.clone(), cmd_rx, &self.services, player_id, &mut client_codec_chain, &mut server_codec_chain).await;

        // ── KickedFromServerEvent (on backend disconnect) ──
        if let ProxyLoopOutcome::BackendDisconnected { ref reason } = outcome {
            let kick_reason = reason.as_deref().unwrap_or("Disconnected");
            let kicked = infrarust_api::events::connection::KickedFromServerEvent::new(
                player_id,
                infrarust_api::types::ServerId::new(routing.config_id.clone()),
                infrarust_api::types::Component::text(kick_reason),
            );
            let _kicked = self.services.event_bus.fire(kicked).await;
            // For now, always disconnect. Phase 4 will handle RedirectTo/SendToLimbo.
        }

        // ── DisconnectEvent (always) ──
        let disconnect = infrarust_api::events::lifecycle::DisconnectEvent {
            player_id,
            username: game_profile.name.clone(),
            last_server: Some(infrarust_api::types::ServerId::new(routing.config_id.clone())),
        };
        let _ = self.services.event_bus.fire(disconnect).await;

        // Cleanup
        let _ = self.services.connection_registry.unregister(&session_id);

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

        client.send_packet(&login_success, &self.services.packet_registry).await?;
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

            let decoded = self.services.packet_registry.decode_frame(
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
                            .disconnect("Backend refused connection", &self.services.packet_registry)
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
