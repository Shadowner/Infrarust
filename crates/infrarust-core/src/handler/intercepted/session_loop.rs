//! Unified session loop for intercepted proxy modes.

use std::sync::Arc;

use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::LimboHandler;
use infrarust_api::types::{Component, PlayerId};
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use infrarust_transport::BackendConnector;
use tokio_util::sync::CancellationToken;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::ConnectCause;

use crate::error::CoreError;
use crate::filter::codec_chain::CodecFilterChain;
use crate::limbo::LIMBO_SWITCH_TARGET;
use crate::limbo::engine::{LimboExitResult, enter_limbo};
use crate::pipeline::types::HandshakeData;
use crate::player::PlayerSession;
use crate::player::commands::CommandInbox;
use crate::services::ProxyServices;
use crate::session::client_bridge::ClientBridge;
use crate::session::proxy_loop::{ProxyLoopOutcome, proxy_loop};
use crate::session::server_join::ServerJoin;
use crate::session::server_switch::{SwitchResult, SwitchTarget};
use crate::util::text::decode_text_component;

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
    mut pending: Pending,
    session: &Arc<PlayerSession>,
    services: &ProxyServices,
    backend_connector: &BackendConnector,
    session_token: CancellationToken,
    commands: &mut CommandInbox,
    client_codec_chain: &mut CodecFilterChain,
    server_codec_chain: &mut CodecFilterChain,
) -> ProxyLoopOutcome {
    let mut mode = initial_mode;

    loop {
        match mode {
            ConnectionMode::Backend(ref mut backend) => {
                session.set_connected_address(backend.server_address().cloned());
                pending.approved = None;
                let outcome = proxy_loop(
                    client,
                    backend,
                    &services.packet_registry,
                    session_token.clone(),
                    commands,
                    services,
                    player_id,
                    client_codec_chain,
                    server_codec_chain,
                    &mut pending.join,
                )
                .await;

                match outcome {
                    ProxyLoopOutcome::SwitchRequested { target }
                        if target.as_str() == LIMBO_SWITCH_TARGET =>
                    {
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
                                let reason =
                                    Component::text("No limbo handlers configured for this server");
                                if let Ok(frame) = crate::player::packets::build_disconnect(
                                    &reason,
                                    version,
                                    &services.packet_registry,
                                ) {
                                    let _ = client.write_frame(&frame).await;
                                }
                                break ProxyLoopOutcome::Kicked { reason };
                            }
                        }
                    }
                    ProxyLoopOutcome::SwitchRequested { target } if target == current_server_id => {
                        tracing::debug!(server = %target, "already on the requested server");
                        continue;
                    }
                    ProxyLoopOutcome::SwitchRequested { target } => {
                        match handle_switch(
                            client,
                            &current_server_id,
                            SwitchTarget::Unapproved {
                                server: target,
                                cause: ConnectCause::Switch,
                            },
                            handshake,
                            game_profile_name,
                            session,
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
                                current_server_id = new_server;
                                pending.join = None;
                                tracing::debug!("re-entering proxy loop after switch");
                                continue;
                            }
                            SwitchAction::Unchanged => continue,
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
                            SwitchAction::Denied(reason) => {
                                tracing::info!(reason = %reason, "server switch denied by event");
                                if let Ok(frame) = crate::player::packets::build_system_chat_message(
                                    &reason,
                                    version,
                                    &services.packet_registry,
                                ) {
                                    let _ = client.write_frame(&frame).await;
                                }
                                continue;
                            }
                            SwitchAction::Error(e) => {
                                tracing::warn!("server switch failed: {e}");
                                let error_msg =
                                    Component::text(format!("Server switch failed: {e}"));
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
                            session,
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
                                current_server_id = new_server;
                                pending.join = None;
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
                session.set_connected_address(None);
                pending.join = None;
                let exit = enter_limbo(
                    client,
                    handlers.clone(),
                    player_id,
                    api_profile.clone(),
                    version,
                    entry_ctx.clone(),
                    services,
                    session_token.clone(),
                    commands,
                )
                .await;

                let gate_target = match entry_ctx {
                    LimboEntryContext::InitialConnection { target_server } => {
                        Some(target_server.clone())
                    }
                    _ => None,
                };
                let from_initial = gate_target.is_some();

                match exit {
                    LimboExitResult::Completed | LimboExitResult::SwitchedTo(_) => {
                        let target = match exit {
                            LimboExitResult::SwitchedTo(ref s) => s.clone(),
                            _ => current_server_id.clone(),
                        };
                        let target = pending.release(target, gate_target.as_ref());
                        match handle_switch(
                            client,
                            &current_server_id,
                            target,
                            handshake,
                            game_profile_name,
                            session,
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
                                current_server_id = new_server;
                                continue;
                            }
                            SwitchAction::Unchanged => {
                                break ProxyLoopOutcome::Error(CoreError::Other(
                                    "a limbo exit kept no server to join".to_string(),
                                ));
                            }
                            SwitchAction::Limbo(handlers, limbo_ctx) => {
                                if from_initial || handlers.is_empty() {
                                    if from_initial {
                                        tracing::warn!(
                                            "skipping re-entry into limbo after initial connection gate"
                                        );
                                    }
                                    break ProxyLoopOutcome::Error(CoreError::Other(
                                        "no limbo handlers left to continue with".to_string(),
                                    ));
                                }
                                mode = ConnectionMode::Limbo(handlers, limbo_ctx);
                                continue;
                            }
                            SwitchAction::Denied(reason) => {
                                tracing::info!(reason = %reason, "switch after limbo denied by event");
                                if let Ok(frame) = crate::player::packets::build_disconnect(
                                    &reason,
                                    version,
                                    &services.packet_registry,
                                ) {
                                    let _ = client.write_frame(&frame).await;
                                }
                                break ProxyLoopOutcome::Kicked { reason };
                            }
                            SwitchAction::Error(e) => {
                                tracing::warn!("switch after limbo failed: {e}");
                                break ProxyLoopOutcome::Error(e);
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
                            break ProxyLoopOutcome::Error(CoreError::Other(
                                "no limbo handlers resolved for a limbo-to-limbo move".to_string(),
                            ));
                        }
                        mode = ConnectionMode::Limbo(
                            handlers,
                            LimboEntryContext::PluginRedirect {
                                from_server: Some(current_server_id.clone()),
                            },
                        );
                        continue;
                    }
                    LimboExitResult::Kicked(reason) => {
                        break ProxyLoopOutcome::Kicked { reason };
                    }
                    LimboExitResult::Timeout | LimboExitResult::ClientDisconnected => {
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

pub(super) struct Pending {
    join: Option<ServerJoin>,
    approved: Option<infrarust_api::types::ServerId>,
}

impl Pending {
    pub(super) const fn join(join: ServerJoin) -> Self {
        Self {
            join: Some(join),
            approved: None,
        }
    }

    pub(super) const fn approved(server: infrarust_api::types::ServerId) -> Self {
        Self {
            join: None,
            approved: Some(server),
        }
    }

    pub(super) const fn nothing() -> Self {
        Self {
            join: None,
            approved: None,
        }
    }

    fn release(
        &mut self,
        target: infrarust_api::types::ServerId,
        gate_target: Option<&infrarust_api::types::ServerId>,
    ) -> SwitchTarget {
        if self.approved.take().as_ref() == Some(&target) {
            return SwitchTarget::Approved(target);
        }
        let cause = if gate_target == Some(&target) {
            ConnectCause::Initial
        } else {
            ConnectCause::LimboExit
        };
        SwitchTarget::Unapproved {
            server: target,
            cause,
        }
    }
}

enum SwitchAction {
    Backend(
        crate::session::backend_bridge::BackendBridge,
        infrarust_api::types::ServerId,
    ),
    Limbo(Vec<Arc<dyn LimboHandler>>, LimboEntryContext),
    Denied(Component),
    Unchanged,
    Error(CoreError),
}

#[allow(clippy::too_many_arguments)]
async fn handle_switch(
    client: &mut ClientBridge,
    current_server: &infrarust_api::types::ServerId,
    target: SwitchTarget,
    handshake: &HandshakeData,
    game_profile_name: &str,
    session: &Arc<PlayerSession>,
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
        session,
        services,
        backend_connector,
        peer_addr,
        real_ip,
        version,
    )
    .await
    {
        Ok(SwitchResult::Backend(success)) => {
            SwitchAction::Backend(success.new_backend, success.new_server_id)
        }
        Ok(SwitchResult::Limbo(handlers, ctx)) => SwitchAction::Limbo(handlers, ctx),
        Ok(SwitchResult::Denied(reason)) => SwitchAction::Denied(reason),
        Ok(SwitchResult::Unchanged) => SwitchAction::Unchanged,
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
    session: &Arc<PlayerSession>,
    version: ProtocolVersion,
    services: &ProxyServices,
    backend_connector: &BackendConnector,
    peer_addr: std::net::SocketAddr,
    real_ip: Option<std::net::IpAddr>,
) -> DisconnectAction {
    let kick_reason = reason.as_deref().map_or_else(
        || Component::text("Disconnected"),
        |raw| decode_text_component(raw.as_bytes(), version, ConnectionState::Play),
    );
    let kicked = infrarust_api::events::connection::KickedFromServerEvent::new(
        player_id,
        current_server_id.clone(),
        kick_reason.clone(),
    );
    let kicked = services.event_bus.fire(kicked).await;

    match kicked.result() {
        infrarust_api::events::connection::KickedFromServerResult::DisconnectPlayer { reason } => {
            if let Ok(frame) =
                crate::player::packets::build_disconnect(reason, version, &services.packet_registry)
            {
                let _ = client.write_frame(&frame).await;
            }
            DisconnectAction::Break(ProxyLoopOutcome::BackendKicked {
                reason: reason.clone(),
            })
        }
        infrarust_api::events::connection::KickedFromServerResult::RedirectTo(server) => {
            match handle_switch(
                client,
                current_server_id,
                SwitchTarget::Unapproved {
                    server: server.clone(),
                    cause: ConnectCause::KickRedirect,
                },
                handshake,
                game_profile_name,
                session,
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
                SwitchAction::Unchanged => DisconnectAction::Break(ProxyLoopOutcome::Error(
                    CoreError::Other("a kick redirect kept no server to join".to_string()),
                )),
                SwitchAction::Limbo(handlers, ctx) => {
                    if handlers.is_empty() {
                        DisconnectAction::Break(ProxyLoopOutcome::Error(CoreError::Other(
                            "no limbo handlers resolved after a kick".to_string(),
                        )))
                    } else {
                        DisconnectAction::SwitchLimbo(handlers, ctx)
                    }
                }
                SwitchAction::Denied(reason) => {
                    tracing::info!(reason = %reason, "redirect after kick denied by event");
                    DisconnectAction::Break(ProxyLoopOutcome::BackendDisconnected {
                        reason: Some(reason.to_plain()),
                    })
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
                        reason: kick_reason,
                    },
                )
            } else {
                tracing::warn!("SendToLimbo but no limbo handlers resolved, disconnecting");
                if let Ok(frame) = crate::player::packets::build_disconnect(
                    &kick_reason,
                    version,
                    &services.packet_registry,
                ) {
                    let _ = client.write_frame(&frame).await;
                }
                DisconnectAction::Break(ProxyLoopOutcome::BackendKicked {
                    reason: kick_reason,
                })
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
