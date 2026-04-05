//! Unified session loop for intercepted proxy modes.

use std::sync::Arc;

use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::LimboHandler;
use infrarust_api::types::PlayerId;
use infrarust_protocol::version::ProtocolVersion;
use infrarust_transport::BackendConnector;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use infrarust_api::event::ResultedEvent;

use crate::error::CoreError;
use crate::filter::codec_chain::CodecFilterChain;
use crate::limbo::engine::{LimboExitResult, enter_limbo};
use crate::pipeline::types::HandshakeData;
use crate::player::PlayerCommand;
use crate::services::ProxyServices;
use crate::session::client_bridge::ClientBridge;
use crate::session::proxy_loop::{ProxyLoopOutcome, proxy_loop};

use super::initial_connect::ConnectionMode;

/// Alternates between Backend (`proxy_loop`) and Limbo (`enter_limbo`),
/// handling server switches, kicks, and limbo transitions.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_session_loop(
    client: &mut ClientBridge,
    initial_mode: ConnectionMode,
    player_id: PlayerId,
    api_profile: &infrarust_api::types::GameProfile,
    game_profile_name: &str,
    handshake: &HandshakeData,
    version: ProtocolVersion,
    peer_addr: std::net::SocketAddr,
    real_ip: Option<std::net::IpAddr>,
    mut current_server_id: infrarust_api::types::ServerId,
    session_id: &uuid::Uuid,
    services: &ProxyServices,
    backend_connector: &BackendConnector,
    session_token: CancellationToken,
    cmd_rx: &mut mpsc::Receiver<PlayerCommand>,
    client_codec_chain: &mut CodecFilterChain,
    server_codec_chain: &mut CodecFilterChain,
) -> ProxyLoopOutcome {
    let mut mode = initial_mode;

    loop {
        match mode {
            ConnectionMode::Backend(ref mut backend) => {
                let outcome = proxy_loop(
                    client,
                    backend,
                    &services.packet_registry,
                    session_token.clone(),
                    cmd_rx,
                    services,
                    player_id,
                    client_codec_chain,
                    server_codec_chain,
                )
                .await;

                match outcome {
                    ProxyLoopOutcome::SwitchRequested { target } if target.as_str() == "$limbo" => {
                        // "$limbo" sentinel: enter limbo for current server's handlers
                        let server_config = services
                            .domain_router
                            .find_by_server_id(current_server_id.as_str());
                        let handler_names = server_config
                            .map(|c| c.limbo_handlers.clone())
                            .unwrap_or_default();
                        match services
                            .limbo_handler_registry
                            .resolve_handlers(&handler_names)
                        {
                            Ok(handlers) if !handlers.is_empty() => {
                                mode = ConnectionMode::Limbo(
                                    handlers,
                                    LimboEntryContext::PluginRedirect {
                                        from_server: Some(current_server_id.clone()),
                                    },
                                );
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
                                    &services.packet_registry,
                                ) {
                                    let _ = client.write_frame(&frame).await;
                                }
                                break ProxyLoopOutcome::ClientDisconnected;
                            }
                        }
                    }
                    ProxyLoopOutcome::SwitchRequested { target } => {
                        match handle_switch(
                            client,
                            &current_server_id,
                            target,
                            handshake,
                            game_profile_name,
                            player_id,
                            api_profile,
                            services,
                            backend_connector,
                            peer_addr,
                            real_ip,
                            version,
                        )
                        .await
                        {
                            SwitchAction::Backend(new_backend, new_server) => {
                                mode = ConnectionMode::Backend(new_backend);
                                if let Some(session) = services.connection_registry.get(session_id)
                                {
                                    session.set_current_server(new_server.clone());
                                }
                                current_server_id = new_server;
                                tracing::debug!("re-entering proxy loop after switch");
                                continue;
                            }
                            SwitchAction::Limbo(handlers, ctx) => {
                                if handlers.is_empty() {
                                    tracing::warn!(
                                        "SendToLimbo during switch but no handlers, staying on current server"
                                    );
                                    continue;
                                }
                                mode = ConnectionMode::Limbo(handlers, ctx);
                                continue;
                            }
                            SwitchAction::Error(e) => {
                                tracing::warn!("server switch failed: {e}");
                                let error_msg = infrarust_api::types::Component::text(format!(
                                    "Server switch failed: {e}"
                                ));
                                if let Ok(frame) = crate::player::packets::build_system_chat_message(
                                    &error_msg,
                                    version,
                                    &services.packet_registry,
                                ) {
                                    let _ = client.write_frame(&frame).await;
                                }
                                continue;
                            }
                        }
                    }
                    ProxyLoopOutcome::BackendDisconnected { reason } => {
                        match handle_backend_disconnect(
                            client,
                            reason,
                            player_id,
                            &current_server_id,
                            handshake,
                            game_profile_name,
                            api_profile,
                            version,
                            services,
                            backend_connector,
                            peer_addr,
                            real_ip,
                        )
                        .await
                        {
                            DisconnectAction::SwitchBackend(new_backend, new_server) => {
                                mode = ConnectionMode::Backend(new_backend);
                                if let Some(session) = services.connection_registry.get(session_id)
                                {
                                    session.set_current_server(new_server.clone());
                                }
                                current_server_id = new_server;
                                continue;
                            }
                            DisconnectAction::SwitchLimbo(handlers, ctx) => {
                                mode = ConnectionMode::Limbo(handlers, ctx);
                                continue;
                            }
                            DisconnectAction::Break(outcome) => break outcome,
                        }
                    }
                    other => break other,
                }
            }
            ConnectionMode::Limbo(ref handlers, ref entry_ctx) => {
                let exit = enter_limbo(
                    client,
                    handlers.clone(),
                    player_id,
                    api_profile.clone(),
                    version,
                    entry_ctx.clone(),
                    &services.packet_registry,
                    services,
                    session_token.clone(),
                )
                .await;

                // Prevent re-entry into limbo after initial connection gate
                let from_initial = matches!(entry_ctx, LimboEntryContext::InitialConnection { .. });

                match exit {
                    LimboExitResult::Completed | LimboExitResult::SwitchedTo(_) => {
                        let target = match exit {
                            LimboExitResult::SwitchedTo(ref s) => s.clone(),
                            _ => current_server_id.clone(),
                        };
                        match handle_switch(
                            client,
                            &current_server_id,
                            target,
                            handshake,
                            game_profile_name,
                            player_id,
                            api_profile,
                            services,
                            backend_connector,
                            peer_addr,
                            real_ip,
                            version,
                        )
                        .await
                        {
                            SwitchAction::Backend(new_backend, new_server) => {
                                mode = ConnectionMode::Backend(new_backend);
                                if let Some(session) = services.connection_registry.get(session_id)
                                {
                                    session.set_current_server(new_server.clone());
                                }
                                current_server_id = new_server;
                                continue;
                            }
                            SwitchAction::Limbo(handlers, limbo_ctx) => {
                                if from_initial || handlers.is_empty() {
                                    if from_initial {
                                        tracing::warn!(
                                            "skipping re-entry into limbo after initial connection gate"
                                        );
                                    }
                                    break ProxyLoopOutcome::ClientDisconnected;
                                }
                                mode = ConnectionMode::Limbo(handlers, limbo_ctx);
                                continue;
                            }
                            SwitchAction::Error(e) => {
                                tracing::warn!("switch after limbo failed: {e}");
                                break ProxyLoopOutcome::ClientDisconnected;
                            }
                        }
                    }
                    LimboExitResult::SendToLimbo(handler_names) => {
                        let handlers = services
                            .limbo_handler_registry
                            .resolve_handlers_lenient(&handler_names);
                        if handlers.is_empty() {
                            tracing::warn!(
                                "limbo-to-limbo but no valid handlers resolved, disconnecting"
                            );
                            break ProxyLoopOutcome::ClientDisconnected;
                        }
                        mode = ConnectionMode::Limbo(
                            handlers,
                            LimboEntryContext::PluginRedirect {
                                from_server: Some(current_server_id.clone()),
                            },
                        );
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
    }
}

enum SwitchAction {
    Backend(
        crate::session::backend_bridge::BackendBridge,
        infrarust_api::types::ServerId,
    ),
    Limbo(Vec<Arc<dyn LimboHandler>>, LimboEntryContext),
    Error(CoreError),
}

#[allow(clippy::too_many_arguments)]
async fn handle_switch(
    client: &mut ClientBridge,
    current_server: &infrarust_api::types::ServerId,
    target: infrarust_api::types::ServerId,
    handshake: &HandshakeData,
    game_profile_name: &str,
    player_id: PlayerId,
    api_profile: &infrarust_api::types::GameProfile,
    services: &ProxyServices,
    backend_connector: &BackendConnector,
    peer_addr: std::net::SocketAddr,
    real_ip: Option<std::net::IpAddr>,
    version: ProtocolVersion,
) -> SwitchAction {
    match crate::session::server_switch::perform_switch(
        client,
        current_server,
        target,
        handshake,
        game_profile_name,
        player_id,
        api_profile,
        services,
        backend_connector,
        peer_addr,
        real_ip,
        version,
    )
    .await
    {
        Ok(crate::session::server_switch::SwitchResult::Backend(success)) => {
            SwitchAction::Backend(success.new_backend, success.new_server_id)
        }
        Ok(crate::session::server_switch::SwitchResult::Limbo(handlers, ctx)) => {
            SwitchAction::Limbo(handlers, ctx)
        }
        Err(e) => SwitchAction::Error(e),
    }
}

enum DisconnectAction {
    SwitchBackend(
        crate::session::backend_bridge::BackendBridge,
        infrarust_api::types::ServerId,
    ),
    SwitchLimbo(Vec<Arc<dyn LimboHandler>>, LimboEntryContext),
    Break(ProxyLoopOutcome),
}

#[allow(clippy::too_many_arguments)]
async fn handle_backend_disconnect(
    client: &mut ClientBridge,
    reason: Option<String>,
    player_id: PlayerId,
    current_server_id: &infrarust_api::types::ServerId,
    handshake: &HandshakeData,
    game_profile_name: &str,
    api_profile: &infrarust_api::types::GameProfile,
    version: ProtocolVersion,
    services: &ProxyServices,
    backend_connector: &BackendConnector,
    peer_addr: std::net::SocketAddr,
    real_ip: Option<std::net::IpAddr>,
) -> DisconnectAction {
    let kick_reason = reason.as_deref().unwrap_or("Disconnected");
    let kicked = infrarust_api::events::connection::KickedFromServerEvent::new(
        player_id,
        current_server_id.clone(),
        infrarust_api::types::Component::text(kick_reason),
    );
    let kicked = services.event_bus.fire(kicked).await;

    match kicked.result() {
        infrarust_api::events::connection::KickedFromServerResult::DisconnectPlayer { reason } => {
            if let Ok(frame) =
                crate::player::packets::build_disconnect(reason, version, &services.packet_registry)
            {
                let _ = client.write_frame(&frame).await;
            }
            DisconnectAction::Break(ProxyLoopOutcome::ClientDisconnected)
        }
        infrarust_api::events::connection::KickedFromServerResult::RedirectTo(server) => {
            match handle_switch(
                client,
                current_server_id,
                server.clone(),
                handshake,
                game_profile_name,
                player_id,
                api_profile,
                services,
                backend_connector,
                peer_addr,
                real_ip,
                version,
            )
            .await
            {
                SwitchAction::Backend(new_backend, new_server) => {
                    DisconnectAction::SwitchBackend(new_backend, new_server)
                }
                SwitchAction::Limbo(handlers, ctx) => {
                    if handlers.is_empty() {
                        DisconnectAction::Break(ProxyLoopOutcome::ClientDisconnected)
                    } else {
                        DisconnectAction::SwitchLimbo(handlers, ctx)
                    }
                }
                SwitchAction::Error(e) => {
                    tracing::warn!("redirect after kick failed: {e}");
                    DisconnectAction::Break(ProxyLoopOutcome::BackendDisconnected {
                        reason: Some(e.to_string()),
                    })
                }
            }
        }
        infrarust_api::events::connection::KickedFromServerResult::SendToLimbo {
            limbo_handlers,
        } => {
            let handler_names = if limbo_handlers.is_empty() {
                services
                    .domain_router
                    .find_by_server_id(current_server_id.as_str())
                    .map(|c| c.limbo_handlers.clone())
                    .unwrap_or_default()
            } else {
                limbo_handlers.clone()
            };
            let handlers = services
                .limbo_handler_registry
                .resolve_handlers_lenient(&handler_names);
            if !handlers.is_empty() {
                DisconnectAction::SwitchLimbo(
                    handlers,
                    LimboEntryContext::KickedFromServer {
                        server: current_server_id.clone(),
                        reason: infrarust_api::types::Component::text(kick_reason),
                    },
                )
            } else {
                tracing::warn!("SendToLimbo but no limbo handlers resolved, disconnecting");
                let kick_component = infrarust_api::types::Component::text(kick_reason);
                if let Ok(frame) = crate::player::packets::build_disconnect(
                    &kick_component,
                    version,
                    &services.packet_registry,
                ) {
                    let _ = client.write_frame(&frame).await;
                }
                DisconnectAction::Break(ProxyLoopOutcome::ClientDisconnected)
            }
        }
        infrarust_api::events::connection::KickedFromServerResult::Notify { message } => {
            if let Ok(frame) = crate::player::packets::build_system_chat_message(
                message,
                version,
                &services.packet_registry,
            ) {
                let _ = client.write_frame(&frame).await;
            }
            DisconnectAction::Break(ProxyLoopOutcome::BackendDisconnected { reason: None })
        }
        _ => DisconnectAction::Break(ProxyLoopOutcome::BackendDisconnected { reason }),
    }
}
