//! `ClientOnly` proxy mode handler.
//!
//! Authenticates the client against Mojang (RSA + AES-128-CFB8 + session server),
//! then connects to the backend in offline mode. The client-side connection is
//! encrypted, the backend-side is plain.

use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::LimboHandler;
use tokio_util::sync::CancellationToken;

use infrarust_protocol::packets::login::{CLoginSuccess, Property, SLoginAcknowledged};
use infrarust_protocol::registry::DecodedPacket;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use infrarust_transport::BackendConnector;

use crate::auth::mojang::MojangAuth;
use crate::error::CoreError;
use crate::limbo::engine::{enter_limbo, LimboExitResult};
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, LoginData, RoutingData};
use crate::player::PlayerSession;
use crate::services::ProxyServices;
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;
use crate::session::proxy_loop::{ProxyLoopOutcome, proxy_loop};

/// Active connection mode within the proxy session loop.
///
/// The loop alternates between modes as the player is switched between
/// backend servers and limbo. Each variant owns the resources needed
/// for that mode.
enum ConnectionMode {
    /// Proxying to a real backend server.
    Backend(BackendBridge),
    /// In limbo (virtual world, no backend).
    Limbo(Vec<Arc<dyn LimboHandler>>, LimboEntryContext),
}

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

        let mut client = ClientBridge::new(ctx.take_stream(), ctx.buffered_data.split(), version);

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

        self.services.event_bus.fire_and_forget_arc(infrarust_api::events::lifecycle::PostLoginEvent {
            profile: api_profile.clone(),
            player_id,
            protocol_version: infrarust_api::types::ProtocolVersion::new(version.0),
        });

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

        let initial_server = infrarust_api::types::ServerId::new(routing.config_id.clone());
        let choose = infrarust_api::events::connection::PlayerChooseInitialServerEvent::new(
            player_id, api_profile.clone(), initial_server.clone(),
        );

        let choose = self.services.event_bus.fire(choose).await;
        let mut initial_mode: Option<ConnectionMode> = None;
        let target_server_id = match choose.result() {
            infrarust_api::events::connection::PlayerChooseInitialServerResult::Allowed => initial_server,
            infrarust_api::events::connection::PlayerChooseInitialServerResult::Redirect(id) => id.clone(),
            infrarust_api::events::connection::PlayerChooseInitialServerResult::SendToLimbo { limbo_handlers } => {
                if let Err(e) = crate::limbo::login::complete_config_for_limbo(
                    &mut client,
                    version,
                    &self.services.packet_registry,
                    &self.services.registry_codec_cache,
                ).await {
                    tracing::warn!("SendToLimbo at initial connect failed: {e}");
                    client.disconnect(&e.to_string(), &self.services.packet_registry).await.ok();
                    return Ok(());
                }

                let handlers = match self.services.limbo_handler_registry.resolve_handlers(limbo_handlers) {
                    Ok(h) if !h.is_empty() => h,
                    _ => {
                        tracing::warn!("SendToLimbo at initial connect but no handlers resolved");
                        client.disconnect("No limbo handlers configured", &self.services.packet_registry).await.ok();
                        return Ok(());
                    }
                };

                initial_mode = Some(ConnectionMode::Limbo(handlers, LimboEntryContext::InitialConnection));
                initial_server
            }
            _ => initial_server,
        };

        if initial_mode.is_none() {
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
            infrarust_api::events::connection::ServerPreConnectResult::SendToLimbo { limbo_handlers } => {
                // Complete config phase for limbo (replay cached or embedded registries)
                if let Err(e) = crate::limbo::login::complete_config_for_limbo(
                    &mut client,
                    version,
                    &self.services.packet_registry,
                    &self.services.registry_codec_cache,
                )
                .await
                {
                    tracing::warn!("SendToLimbo at initial connect failed: {e}");
                    client.disconnect(&e.to_string(), &self.services.packet_registry).await.ok();
                    return Ok(());
                }

                let handler_names = if limbo_handlers.is_empty() {
                    server_config.limbo_handlers.clone()
                } else {
                    limbo_handlers.clone()
                };
                let handlers = self.services.limbo_handler_registry
                    .resolve_handlers_lenient(&handler_names);
                if handlers.is_empty() {
                    tracing::warn!("SendToLimbo at initial connect but no handlers resolved");
                    client.disconnect("No limbo handlers configured", &self.services.packet_registry).await.ok();
                    return Ok(());
                }

                initial_mode = Some(ConnectionMode::Limbo(handlers, LimboEntryContext::InitialConnection));
            }
            _ => {} // ConnectTo, VirtualBackend — Phase 4
        }
        } // end if initial_mode.is_none()

        // If limbo handlers are configured and no event already set a mode,
        // enter limbo before attempting backend connection.
        if initial_mode.is_none() && !server_config.limbo_handlers.is_empty() {
            if let Err(e) = crate::limbo::login::complete_config_for_limbo(
                &mut client,
                version,
                &self.services.packet_registry,
                &self.services.registry_codec_cache,
            ).await {
                tracing::warn!("limbo config phase failed: {e}");
                client.disconnect(&e.to_string(), &self.services.packet_registry).await.ok();
                return Ok(());
            }

            let handlers = self.services.limbo_handler_registry
                .resolve_handlers_lenient(&server_config.limbo_handlers);
            if !handlers.is_empty() {
                initial_mode = Some(ConnectionMode::Limbo(handlers, LimboEntryContext::InitialConnection));
            }
        }

        // Determine initial connection mode: either connect to backend or enter limbo
        let mut mode = if let Some(limbo_mode) = initial_mode {
            limbo_mode
        } else {
            // Connect to backend (normal flow)
            match self
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
                Ok(backend_conn) => {
                    let mut backend = BackendBridge::new(backend_conn.into_stream(), version);

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
                    if let Err(e) = backend.consume_backend_login(&self.services.packet_registry, version).await {
                        client
                            .disconnect("Backend refused connection", &self.services.packet_registry)
                            .await
                            .ok();
                        return Err(e);
                    }

                    // For 1.20.2+: send LoginAcknowledged to backend to transition it to Config
                    if version.no_less_than(ProtocolVersion::V1_20_2) {
                        let ack = SLoginAcknowledged;
                        backend.send_packet(&ack, &self.services.packet_registry).await?;
                        backend.set_state(ConnectionState::Config);
                        tracing::debug!("backend LoginAcknowledged → Config");
                    }

                    self.services.event_bus.fire_and_forget_arc(infrarust_api::events::connection::ServerConnectedEvent {
                        player_id,
                        server: target_server_id.clone(),
                    });

                    ConnectionMode::Backend(backend)
                }
                Err(e) => {
                    // If limbo handlers are configured, fall back to limbo
                    if !server_config.limbo_handlers.is_empty() {
                        tracing::info!(
                            server = %routing.config_id,
                            error = %e,
                            "backend unreachable, falling back to limbo"
                        );

                        // Complete config phase for limbo
                        if let Err(config_err) = crate::limbo::login::complete_config_for_limbo(
                            &mut client,
                            version,
                            &self.services.packet_registry,
                            &self.services.registry_codec_cache,
                        ).await {
                            tracing::warn!("limbo fallback config phase failed: {config_err}");
                            client.disconnect(&config_err.to_string(), &self.services.packet_registry).await.ok();
                            return Ok(());
                        }

                        let handlers = self.services.limbo_handler_registry
                            .resolve_handlers_lenient(&server_config.limbo_handlers);
                        if !handlers.is_empty() {
                            ConnectionMode::Limbo(handlers, LimboEntryContext::KickedFromServer {
                                server: target_server_id.clone(),
                                reason: infrarust_api::types::Component::text(
                                    format!("Backend unreachable: {e}")
                                ),
                            })
                        } else {
                            let msg = server_config.effective_disconnect_message();
                            client.disconnect(msg, &self.services.packet_registry).await.ok();
                            return Ok(());
                        }
                    } else {
                        tracing::warn!(
                            server = %routing.config_id,
                            error = %e,
                            "backend unreachable, sending disconnect to client"
                        );
                        let msg = server_config.effective_disconnect_message();
                        client.disconnect(msg, &self.services.packet_registry).await.ok();
                        return Ok(());
                    }
                }
            }
        };

        // Session registration (shared by both backend and limbo paths)
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
        super::helpers::record_session_start(&self.metrics, &routing.config_id, "client_only");

        // Build codec filter chains
        let (mut client_codec_chain, mut server_codec_chain) =
            crate::filter::codec_chain::build_codec_chains(
                &self.services.codec_filter_registry,
                infrarust_api::types::ProtocolVersion::new(handshake.protocol_version.0),
                player_id.as_u64(),
                ctx.peer_addr,
                Some(ctx.client_ip),
            );

        // Proxy loop with server switch and limbo support
        let mut cmd_rx = cmd_rx;
        let mut current_server_id = target_server_id;

        let outcome = loop {
            match mode {
                ConnectionMode::Backend(ref mut backend) => {
                    let outcome = proxy_loop(
                        &mut client,
                        backend,
                        &self.services.packet_registry,
                        session_token.clone(),
                        &mut cmd_rx,
                        &self.services,
                        player_id,
                        &mut client_codec_chain,
                        &mut server_codec_chain,
                    )
                    .await;

                    match outcome {
                        ProxyLoopOutcome::SwitchRequested { target } if target.as_str() == "$limbo" => {
                            // Sentinel: enter limbo for current server's handlers
                            let server_config = self
                                .services
                                .domain_router
                                .find_by_server_id(current_server_id.as_str());
                            let handler_names = server_config
                                .map(|c| c.limbo_handlers.clone())
                                .unwrap_or_default();
                            match self
                                .services
                                .limbo_handler_registry
                                .resolve_handlers(&handler_names)
                            {
                                Ok(handlers) if !handlers.is_empty() => {
                                    mode = ConnectionMode::Limbo(handlers, LimboEntryContext::PluginRedirect {
                                        from_server: Some(current_server_id.clone()),
                                    });
                                    continue;
                                }
                                _ => {
                                    tracing::warn!("no limbo handlers configured, disconnecting");
                                    let reason = infrarust_api::types::Component::text(
                                        "No limbo handlers configured for this server",
                                    );
                                    if let Ok(frame) = crate::player::packets::build_disconnect(
                                        &reason,
                                        version,
                                        &self.services.packet_registry,
                                    ) {
                                        let _ = client.write_frame(&frame).await;
                                    }
                                    break ProxyLoopOutcome::ClientDisconnected;
                                }
                            }
                        }
                        ProxyLoopOutcome::SwitchRequested { target } => {
                            match crate::session::server_switch::perform_switch(
                                &mut client,
                                &current_server_id,
                                target,
                                &handshake,
                                &game_profile.name,
                                player_id,
                                &api_profile,
                                &self.services,
                                &self.backend_connector,
                                ctx.peer_addr,
                                version,
                            )
                            .await
                            {
                                Ok(crate::session::server_switch::SwitchResult::Backend(success)) => {
                                    mode = ConnectionMode::Backend(success.new_backend);
                                    if let Some(session) =
                                        self.services.connection_registry.get(&session_id)
                                    {
                                        session
                                            .set_current_server(success.new_server_id.clone());
                                    }
                                    current_server_id = success.new_server_id;
                                    tracing::debug!("re-entering proxy loop after switch");
                                    continue;
                                }
                                Ok(crate::session::server_switch::SwitchResult::Limbo(handlers, ctx)) => {
                                    if handlers.is_empty() {
                                        tracing::warn!("SendToLimbo during switch but no handlers, staying on current server");
                                        continue;
                                    }
                                    mode = ConnectionMode::Limbo(handlers, ctx);
                                    continue;
                                }
                                Err(e) => {
                                    tracing::warn!("server switch failed: {e}");
                                    let error_msg = infrarust_api::types::Component::text(
                                        &format!("Server switch failed: {e}"),
                                    );
                                    if let Ok(frame) =
                                        crate::player::packets::build_system_chat_message(
                                            &error_msg,
                                            version,
                                            &self.services.packet_registry,
                                        )
                                    {
                                        let _ = client.write_frame(&frame).await;
                                    }
                                    continue;
                                }
                            }
                        }
                        ProxyLoopOutcome::BackendDisconnected { reason } => {
                            let kick_reason = reason.as_deref().unwrap_or("Disconnected");
                            let kicked =
                                infrarust_api::events::connection::KickedFromServerEvent::new(
                                    player_id,
                                    current_server_id.clone(),
                                    infrarust_api::types::Component::text(kick_reason),
                                );
                            let kicked = self.services.event_bus.fire(kicked).await;

                            match kicked.result() {
                                infrarust_api::events::connection::KickedFromServerResult::DisconnectPlayer { reason } => {
                                    if let Ok(frame) = crate::player::packets::build_disconnect(
                                        reason,
                                        version,
                                        &self.services.packet_registry,
                                    ) {
                                        let _ = client.write_frame(&frame).await;
                                    }
                                    break ProxyLoopOutcome::ClientDisconnected;
                                }
                                infrarust_api::events::connection::KickedFromServerResult::RedirectTo(server) => {
                                    match crate::session::server_switch::perform_switch(
                                        &mut client,
                                        &current_server_id,
                                        server.clone(),
                                        &handshake,
                                        &game_profile.name,
                                        player_id,
                                        &api_profile,
                                        &self.services,
                                        &self.backend_connector,
                                        ctx.peer_addr,
                                        version,
                                    )
                                    .await
                                    {
                                        Ok(crate::session::server_switch::SwitchResult::Backend(success)) => {
                                            mode = ConnectionMode::Backend(success.new_backend);
                                            if let Some(session) =
                                                self.services.connection_registry.get(&session_id)
                                            {
                                                session.set_current_server(
                                                    success.new_server_id.clone(),
                                                );
                                            }
                                            current_server_id = success.new_server_id;
                                            continue;
                                        }
                                        Ok(crate::session::server_switch::SwitchResult::Limbo(handlers, ctx)) => {
                                            if handlers.is_empty() {
                                                break ProxyLoopOutcome::ClientDisconnected;
                                            }
                                            mode = ConnectionMode::Limbo(handlers, ctx);
                                            continue;
                                        }
                                        Err(e) => {
                                            tracing::warn!("redirect after kick failed: {e}");
                                            break ProxyLoopOutcome::BackendDisconnected {
                                                reason: Some(e.to_string()),
                                            };
                                        }
                                    }
                                }
                                infrarust_api::events::connection::KickedFromServerResult::SendToLimbo { limbo_handlers } => {
                                    let handler_names = if limbo_handlers.is_empty() {
                                        self.services
                                            .domain_router
                                            .find_by_server_id(current_server_id.as_str())
                                            .map(|c| c.limbo_handlers.clone())
                                            .unwrap_or_default()
                                    } else {
                                        limbo_handlers.clone()
                                    };
                                    let handlers = self.services.limbo_handler_registry
                                        .resolve_handlers_lenient(&handler_names);
                                    if !handlers.is_empty() {
                                        mode = ConnectionMode::Limbo(handlers, LimboEntryContext::KickedFromServer {
                                            server: current_server_id.clone(),
                                            reason: infrarust_api::types::Component::text(kick_reason),
                                        });
                                        continue;
                                    } else {
                                        tracing::warn!(
                                            "SendToLimbo but no limbo handlers resolved, disconnecting"
                                        );
                                        let kick_component = infrarust_api::types::Component::text(kick_reason);
                                        if let Ok(frame) = crate::player::packets::build_disconnect(
                                            &kick_component,
                                            version,
                                            &self.services.packet_registry,
                                        ) {
                                            let _ = client.write_frame(&frame).await;
                                        }
                                        break ProxyLoopOutcome::ClientDisconnected;
                                    }
                                }
                                infrarust_api::events::connection::KickedFromServerResult::Notify { message } => {
                                    if let Ok(frame) =
                                        crate::player::packets::build_system_chat_message(
                                            message,
                                            version,
                                            &self.services.packet_registry,
                                        )
                                    {
                                        let _ = client.write_frame(&frame).await;
                                    }
                                    break ProxyLoopOutcome::BackendDisconnected { reason: None };
                                }
                                _ => {
                                    break ProxyLoopOutcome::BackendDisconnected { reason };
                                }
                            }
                        }
                        other => break other,
                    }
                }
                ConnectionMode::Limbo(ref handlers, ref entry_ctx) => {
                    let exit = enter_limbo(
                        &mut client,
                        handlers.clone(),
                        player_id,
                        api_profile.clone(),
                        version,
                        entry_ctx.clone(),
                        &self.services.packet_registry,
                        &self.services,
                        session_token.clone(),
                    )
                    .await;

                    // Track whether this was an initial-connection limbo so we can
                    // prevent re-entry into limbo after perform_switch.
                    let from_initial = matches!(entry_ctx, LimboEntryContext::InitialConnection);

                    match exit {
                        LimboExitResult::Completed | LimboExitResult::SwitchedTo(_) => {
                            let target = match exit {
                                LimboExitResult::SwitchedTo(ref s) => s.clone(),
                                _ => current_server_id.clone(),
                            };
                            match crate::session::server_switch::perform_switch(
                                &mut client,
                                &current_server_id,
                                target,
                                &handshake,
                                &game_profile.name,
                                player_id,
                                &api_profile,
                                &self.services,
                                &self.backend_connector,
                                ctx.peer_addr,
                                version,
                            )
                            .await
                            {
                                Ok(crate::session::server_switch::SwitchResult::Backend(success)) => {
                                    mode = ConnectionMode::Backend(success.new_backend);
                                    if let Some(session) =
                                        self.services.connection_registry.get(&session_id)
                                    {
                                        session
                                            .set_current_server(success.new_server_id.clone());
                                    }
                                    current_server_id = success.new_server_id;
                                    continue;
                                }
                                Ok(crate::session::server_switch::SwitchResult::Limbo(handlers, limbo_ctx)) => {
                                    if from_initial || handlers.is_empty() {
                                        if from_initial {
                                            tracing::warn!("skipping re-entry into limbo after initial connection gate");
                                        }
                                        break ProxyLoopOutcome::ClientDisconnected;
                                    }
                                    mode = ConnectionMode::Limbo(handlers, limbo_ctx);
                                    continue;
                                }
                                Err(e) => {
                                    tracing::warn!("switch after limbo failed: {e}");
                                    break ProxyLoopOutcome::ClientDisconnected;
                                }
                            }
                        }
                        LimboExitResult::SendToLimbo(handler_names) => {
                            let handlers = self.services.limbo_handler_registry
                                .resolve_handlers_lenient(&handler_names);
                            if handlers.is_empty() {
                                tracing::warn!("limbo-to-limbo but no valid handlers resolved, disconnecting");
                                break ProxyLoopOutcome::ClientDisconnected;
                            }
                            mode = ConnectionMode::Limbo(handlers, LimboEntryContext::PluginRedirect {
                                from_server: Some(current_server_id.clone()),
                            });
                            continue;
                        }
                        LimboExitResult::Kicked | LimboExitResult::Timeout => {
                            break ProxyLoopOutcome::ClientDisconnected;
                        }
                        LimboExitResult::ClientDisconnected => {
                            break ProxyLoopOutcome::ClientDisconnected;
                        }
                        LimboExitResult::Shutdown => {
                            break ProxyLoopOutcome::Shutdown;
                        }
                    }
                }
            }
        };

        super::helpers::fire_disconnect_event(
            &self.services.event_bus,
            player_id,
            game_profile.name.clone(),
            Some(current_server_id.clone()),
        ).await;

        // Cleanup
        let _ = self.services.connection_registry.unregister(&session_id);

        // Record end metrics
        #[cfg(feature = "telemetry")]
        super::helpers::record_session_end(&self.metrics, ctx.connection_duration(), &routing.config_id, "client_only");

        super::helpers::log_proxy_loop_outcome(&session_id, &outcome);

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

}
