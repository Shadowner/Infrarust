//! Offline proxy mode handler.
//!
//! No authentication — packets are parsed and forwarded transparently.
//! The proxy activates compression and manages state transitions
//! (Login → Config → Play) but does not verify player identity.

use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use tokio_util::sync::CancellationToken;

use infrarust_transport::BackendConnector;

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, LoginData, RoutingData};
use crate::player::PlayerSession;
use crate::services::ProxyServices;
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;
use crate::session::proxy_loop::{ProxyLoopOutcome, proxy_loop};

/// Handles connections in Offline proxy mode.
///
/// Flow: create bridges → send initial packets → `proxy_loop` → cleanup.
pub struct OfflineHandler {
    backend_connector: Arc<BackendConnector>,
    services: ProxyServices,
    #[cfg(feature = "telemetry")]
    metrics: Option<Arc<crate::telemetry::ProxyMetrics>>,
}

impl OfflineHandler {
    /// Creates a new offline handler.
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

    /// Handles an offline-mode connection.
    ///
    /// # Errors
    /// Returns `CoreError` on backend connection failure or I/O errors.
    #[tracing::instrument(name = "proxy.session", skip_all, fields(mode = "offline"))]
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
        let version = handshake.protocol_version;

        // 1. Create client bridge
        let mut client = ClientBridge::new(
            ctx.take_stream(),
            ctx.buffered_data.split(),
            version,
        );

        // Build player_id and api_profile early for events
        let player_uuid = login_data
            .as_ref()
            .and_then(|d| d.player_uuid)
            .unwrap_or_else(uuid::Uuid::new_v4);
        let username = login_data
            .as_ref()
            .map(|d| d.username.clone())
            .unwrap_or_default();
        let player_id = infrarust_api::types::PlayerId::new(player_uuid.as_u128() as u64);
        let api_profile = infrarust_api::types::GameProfile {
            uuid: player_uuid,
            username: username.clone(),
            properties: vec![],
        };

        // ── PreLoginEvent ──
        let pre_login = infrarust_api::events::lifecycle::PreLoginEvent::new(
            api_profile.clone(),
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

        // ── PostLoginEvent (fire-and-forget) ──
        self.services.event_bus.fire_and_forget_arc(infrarust_api::events::lifecycle::PostLoginEvent {
            profile: api_profile.clone(),
            player_id,
            protocol_version: infrarust_api::types::ProtocolVersion::new(version.0),
        });

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

        // TODO: Phase 4 — resolve target_server_id to addresses for backend connection

        // 2. Connect to backend
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

        // 3. Create backend bridge
        let mut backend = BackendBridge::new(backend_conn.into_stream());

        // 4. Send initial packets (handshake + login start with domain rewrite)
        backend
            .send_initial_packets(&handshake, server_config)
            .await?;

        // ── ServerConnectedEvent (fire-and-forget) ──
        self.services.event_bus.fire_and_forget_arc(infrarust_api::events::connection::ServerConnectedEvent {
            player_id,
            server: target_server_id.clone(),
        });

        // 5. Register session
        let session_token = CancellationToken::new();
        let (cmd_tx, cmd_rx) = PlayerSession::channel();

        let player_session = Arc::new(PlayerSession::new(
            player_id,
            api_profile.clone(),
            infrarust_api::types::ProtocolVersion::new(version.0),
            ctx.peer_addr,
            Some(infrarust_api::types::ServerId::new(routing.config_id.clone())),
            true, // active: Offline mode supports packet injection
            cmd_tx,
            session_token.clone(),
        ));

        let session_id = self.services.connection_registry.register(player_session);

        tracing::info!(
            session = %session_id,
            server = %routing.config_id,
            username = ?login_data.as_ref().map(|d| &d.username),
            mode = "offline",
            "session started"
        );

        // Record metrics
        #[cfg(feature = "telemetry")]
        if let Some(ref metrics) = self.metrics {
            metrics.record_connection_start(&routing.config_id, "offline");
            metrics.record_player_join(&routing.config_id);
        }

        // 6. Combine shutdown tokens
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

        // 7. Build codec filter chains
        let (mut client_codec_chain, mut server_codec_chain) =
            crate::filter::codec_chain::build_codec_chains(
                &self.services.codec_filter_registry,
                infrarust_api::types::ProtocolVersion::new(version.0),
                player_id.as_u64(),
                ctx.peer_addr,
                Some(ctx.client_ip),
            );

        // 8. Proxy loop (Login → Config → Play)
        let outcome =
            proxy_loop(&mut client, &mut backend, &self.services.packet_registry, combined_shutdown, cmd_rx, &self.services, player_id, &mut client_codec_chain, &mut server_codec_chain).await;

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
            username: username.clone(),
            last_server: Some(infrarust_api::types::ServerId::new(routing.config_id.clone())),
        };
        let _ = self.services.event_bus.fire(disconnect).await;

        // 8. Cleanup
        let _ = self.services.connection_registry.unregister(&session_id);

        // Record end metrics
        #[cfg(feature = "telemetry")]
        if let Some(ref metrics) = self.metrics {
            let duration_secs = ctx.connected_at.elapsed().as_secs_f64();
            metrics.record_connection_end(duration_secs, &routing.config_id, "offline");
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
}
