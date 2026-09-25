//! Unified session loop for intercepted proxy modes.

use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::{
    ConnectCause, KickCause, KickedFromServerEvent, KickedFromServerResult,
};
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::LimboHandler;
use infrarust_api::player::Player;
use infrarust_api::types::{Component, GameProfile, PlayerId, ServerId};
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use infrarust_transport::BackendConnector;
use tokio_util::sync::CancellationToken;

use crate::error::CoreError;
use crate::filter::codec_chain::CodecFilterChain;
use crate::limbo::LIMBO_SWITCH_TARGET;
use crate::limbo::engine::{LimboExitResult, enter_limbo};
use crate::pipeline::types::HandshakeData;
use crate::player::PlayerSession;
use crate::player::commands::CommandInbox;
use crate::services::ProxyServices;
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;
use crate::session::kick::Kick;
use crate::session::proxy_loop::{ProxyLoopOutcome, proxy_loop};
use crate::session::server_join::ServerJoin;
use crate::session::server_switch::{SwitchResult, SwitchTarget};

use super::initial_connect::ConnectionMode;

const MAX_KICK_REDIRECTS: usize = 3;
const UNREACHABLE_MESSAGE: &str = "Server is currently unreachable. Please try again later.";

struct Route<'a> {
    profile: &'a GameProfile,
    game_profile_name: &'a str,
    handshake: &'a HandshakeData,
    version: ProtocolVersion,
    peer_addr: std::net::SocketAddr,
    real_ip: Option<std::net::IpAddr>,
    session: &'a Arc<PlayerSession>,
    services: &'a ProxyServices,
    backend_connector: &'a BackendConnector,
}

/// Alternates between Backend (`proxy_loop`) and Limbo (`enter_limbo`),
/// handling server switches, kicks, and limbo transitions.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_session_loop(
    client: &mut ClientBridge,
    initial_mode: ConnectionMode,
    player_id: PlayerId,
    api_profile: &GameProfile,
    game_profile_name: &str,
    handshake: &HandshakeData,
    version: ProtocolVersion,
    peer_addr: std::net::SocketAddr,
    real_ip: Option<std::net::IpAddr>,
    mut current_server_id: ServerId,
    mut pending: Pending,
    session: &Arc<PlayerSession>,
    services: &ProxyServices,
    backend_connector: &BackendConnector,
    session_token: CancellationToken,
    commands: &mut CommandInbox,
    client_codec_chain: &mut CodecFilterChain,
    server_codec_chain: &mut CodecFilterChain,
) -> ProxyLoopOutcome {
    let route = Route {
        profile: api_profile,
        game_profile_name,
        handshake,
        version,
        peer_addr,
        real_ip,
        session,
        services,
        backend_connector,
    };
    let mut mode = initial_mode;

    loop {
        if session_token.is_cancelled() && !matches!(mode, ConnectionMode::Backend(_)) {
            break ProxyLoopOutcome::Shutdown;
        }
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
                        let handler_names = server_limbo_handlers(&route, &current_server_id);
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
                                client
                                    .disconnect(&reason, &services.packet_registry)
                                    .await
                                    .ok();
                                break ProxyLoopOutcome::Kicked { reason };
                            }
                        }
                    }
                    ProxyLoopOutcome::SwitchRequested { target } if target == current_server_id => {
                        tracing::debug!(server = %target, "already on the requested server");
                        continue;
                    }
                    ProxyLoopOutcome::SwitchRequested { target } => {
                        let request = SwitchTarget::Unapproved {
                            server: target,
                            cause: ConnectCause::Switch,
                        };
                        let settled = match switch(&route, client, &current_server_id, request)
                            .await
                        {
                            SwitchAction::Backend(backend, server) => {
                                Settled::Backend(backend, server)
                            }
                            SwitchAction::Unchanged => Settled::Stay,
                            SwitchAction::Limbo(handlers, _) if handlers.is_empty() => {
                                tracing::warn!(
                                    "SendToLimbo during switch but no handlers, staying on current server"
                                );
                                Settled::Stay
                            }
                            SwitchAction::Limbo(handlers, entry) => {
                                Settled::Limbo(handlers, entry, current_server_id.clone())
                            }
                            SwitchAction::Denied(reason) => {
                                tracing::info!(reason = %reason, "server switch denied by event");
                                chat(&route, client, &reason).await;
                                Settled::Stay
                            }
                            SwitchAction::Failed(kick) => {
                                settle(&route, client, &current_server_id, kick, true).await
                            }
                            SwitchAction::Error(e) => {
                                tracing::warn!("server switch failed: {e}");
                                let message = Component::text(format!("Server switch failed: {e}"));
                                chat(&route, client, &message).await;
                                Settled::Stay
                            }
                        };
                        match settled {
                            Settled::Stay => continue,
                            Settled::Backend(backend, server) => {
                                mode = ConnectionMode::Backend(backend);
                                current_server_id = server;
                                pending.join = None;
                            }
                            Settled::Limbo(handlers, entry, server) => {
                                mode = ConnectionMode::Limbo(handlers, entry);
                                current_server_id = server;
                            }
                            Settled::End(outcome) => break outcome,
                        }
                    }
                    ProxyLoopOutcome::BackendKick(packet) => {
                        let during_connect = pending.join.take().is_some();
                        mode = ConnectionMode::Kicked(Kick::from_packet(
                            current_server_id.clone(),
                            *packet,
                            during_connect,
                        ));
                    }
                    ProxyLoopOutcome::BackendDisconnected { reason } => {
                        tracing::debug!(?reason, "backend connection lost");
                        let during_connect = pending.join.take().is_some();
                        mode = ConnectionMode::Kicked(Kick::lost(
                            current_server_id.clone(),
                            during_connect,
                        ));
                    }
                    other => break other,
                }
            }
            ConnectionMode::Kicked(ref kick) => {
                session.set_connected_address(None);
                pending.join = None;
                match settle(&route, client, &current_server_id, kick.clone(), false).await {
                    Settled::Backend(backend, server) => {
                        mode = ConnectionMode::Backend(backend);
                        current_server_id = server;
                    }
                    Settled::Limbo(handlers, entry, server) => {
                        mode = ConnectionMode::Limbo(handlers, entry);
                        current_server_id = server;
                    }
                    Settled::End(outcome) => break outcome,
                    Settled::Stay => {
                        break ProxyLoopOutcome::Error(CoreError::Other(
                            "a kicked player has no server to stay on".to_string(),
                        ));
                    }
                }
            }
            ConnectionMode::Limbo(ref handlers, ref entry_ctx) => {
                session.set_connected_address(None);
                pending.join = None;
                if let Err(e) = enter_play(&route, client).await {
                    tracing::warn!("could not bring the client into play for limbo: {e}");
                    client
                        .disconnect(&Component::text(e.to_string()), &services.packet_registry)
                        .await
                        .ok();
                    break ProxyLoopOutcome::Error(e);
                }
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
                    LimboExitResult::Completed | LimboExitResult::SwitchedTo(_)
                        if session_token.is_cancelled() =>
                    {
                        break ProxyLoopOutcome::Shutdown;
                    }
                    LimboExitResult::Completed | LimboExitResult::SwitchedTo(_) => {
                        let target = match exit {
                            LimboExitResult::SwitchedTo(ref s) => s.clone(),
                            _ => current_server_id.clone(),
                        };
                        let target = pending.release(target, gate_target.as_ref());
                        match switch(&route, client, &current_server_id, target).await {
                            SwitchAction::Backend(new_backend, new_server) => {
                                mode = ConnectionMode::Backend(new_backend);
                                current_server_id = new_server;
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
                            }
                            SwitchAction::Denied(reason) => {
                                tracing::info!(reason = %reason, "switch after limbo denied by event");
                                client
                                    .disconnect(&reason, &services.packet_registry)
                                    .await
                                    .ok();
                                break ProxyLoopOutcome::Kicked { reason };
                            }
                            SwitchAction::Failed(kick) => {
                                mode = ConnectionMode::Kicked(kick);
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
    approved: Option<ServerId>,
}

impl Pending {
    pub(super) const fn join(join: ServerJoin) -> Self {
        Self {
            join: Some(join),
            approved: None,
        }
    }

    pub(super) const fn approved(server: ServerId) -> Self {
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

    fn release(&mut self, target: ServerId, gate_target: Option<&ServerId>) -> SwitchTarget {
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
    Backend(BackendBridge, ServerId),
    Limbo(Vec<Arc<dyn LimboHandler>>, LimboEntryContext),
    Denied(Component),
    Unchanged,
    Failed(Kick),
    Error(CoreError),
}

async fn switch(
    route: &Route<'_>,
    client: &mut ClientBridge,
    current_server: &ServerId,
    target: SwitchTarget,
) -> SwitchAction {
    match crate::session::server_switch::perform_switch(
        client,
        current_server,
        target,
        route.handshake,
        route.game_profile_name,
        route.session,
        route.services,
        route.backend_connector,
        route.peer_addr,
        route.real_ip,
        route.version,
    )
    .await
    {
        Ok(SwitchResult::Backend(success)) => {
            SwitchAction::Backend(success.new_backend, success.new_server_id)
        }
        Ok(SwitchResult::Limbo(handlers, ctx)) => SwitchAction::Limbo(handlers, ctx),
        Ok(SwitchResult::Denied(reason)) => SwitchAction::Denied(reason),
        Ok(SwitchResult::Unchanged) => SwitchAction::Unchanged,
        Ok(SwitchResult::Failed(kick)) => SwitchAction::Failed(kick),
        Err(e) => SwitchAction::Error(e),
    }
}

enum Settled {
    Stay,
    Backend(BackendBridge, ServerId),
    Limbo(Vec<Arc<dyn LimboHandler>>, LimboEntryContext, ServerId),
    End(ProxyLoopOutcome),
}

async fn settle(
    route: &Route<'_>,
    client: &mut ClientBridge,
    current_server: &ServerId,
    mut kick: Kick,
    mut can_stay: bool,
) -> Settled {
    let mut redirects = 0;
    loop {
        if route.session.shutdown_token().is_cancelled() {
            return Settled::End(ProxyLoopOutcome::Shutdown);
        }
        can_stay &= !kick.stranded;
        match fire_kicked(route, &kick, can_stay).await {
            KickedFromServerResult::RedirectTo(target) if redirects < MAX_KICK_REDIRECTS => {
                redirects += 1;
                if let Err(e) = leave_login(route, client).await {
                    return Settled::End(ProxyLoopOutcome::Error(e));
                }
                let request = SwitchTarget::Unapproved {
                    server: target,
                    cause: ConnectCause::KickRedirect,
                };
                match switch(route, client, current_server, request).await {
                    SwitchAction::Backend(backend, server) => {
                        return Settled::Backend(backend, server);
                    }
                    SwitchAction::Limbo(handlers, entry) if !handlers.is_empty() => {
                        return Settled::Limbo(handlers, entry, current_server.clone());
                    }
                    SwitchAction::Failed(next) => kick = next,
                    SwitchAction::Denied(reason) => {
                        return notify(route, client, &kick, reason, can_stay).await;
                    }
                    SwitchAction::Limbo(..) | SwitchAction::Unchanged => {
                        return fall_back(route, client, &kick, can_stay).await;
                    }
                    SwitchAction::Error(e) => {
                        tracing::warn!(server = %kick.server, "kick redirect failed: {e}");
                        return fall_back(route, client, &kick, can_stay).await;
                    }
                }
            }
            KickedFromServerResult::RedirectTo(target) => {
                tracing::warn!(
                    server = %kick.server,
                    target = %target,
                    "giving up after {MAX_KICK_REDIRECTS} kick redirects"
                );
                return Settled::End(disconnect(route, client, &kick, None).await);
            }
            KickedFromServerResult::SendToLimbo { limbo_handlers } => {
                return to_limbo(route, client, &kick, limbo_handlers, can_stay).await;
            }
            KickedFromServerResult::Notify { message } => {
                return notify(route, client, &kick, message, can_stay).await;
            }
            KickedFromServerResult::DisconnectPlayer { reason } => {
                return Settled::End(disconnect(route, client, &kick, reason).await);
            }
            _ => return Settled::End(disconnect(route, client, &kick, None).await),
        }
    }
}

async fn fire_kicked(route: &Route<'_>, kick: &Kick, can_stay: bool) -> KickedFromServerResult {
    let player = Arc::clone(route.session) as Arc<dyn Player>;
    let previous_server = if kick.during_connect {
        player.current_server()
    } else {
        route.session.previous_server()
    };
    let event = KickedFromServerEvent::new(
        player,
        kick.server.clone(),
        kick.reason(),
        kick.cause.clone(),
        kick.during_connect,
        previous_server,
        default_result(route, kick, can_stay),
    );
    route.services.event_bus.fire(event).await.result().clone()
}

fn default_result(route: &Route<'_>, kick: &Kick, can_stay: bool) -> KickedFromServerResult {
    if !kick.during_connect {
        return KickedFromServerResult::DisconnectPlayer { reason: None };
    }
    if can_stay {
        return KickedFromServerResult::Notify {
            message: shown_reason(route, kick),
        };
    }
    let limbo_handlers = server_limbo_handlers(route, &kick.server);
    let resolvable = !limbo_handlers.is_empty()
        && !route
            .services
            .limbo_handler_registry
            .resolve_handlers_lenient(&limbo_handlers)
            .is_empty();
    if resolvable {
        KickedFromServerResult::SendToLimbo { limbo_handlers }
    } else {
        KickedFromServerResult::DisconnectPlayer { reason: None }
    }
}

async fn notify(
    route: &Route<'_>,
    client: &mut ClientBridge,
    kick: &Kick,
    message: Component,
    can_stay: bool,
) -> Settled {
    if can_stay {
        chat(route, client, &message).await;
        return Settled::Stay;
    }
    Settled::End(disconnect(route, client, kick, Some(message)).await)
}

async fn fall_back(
    route: &Route<'_>,
    client: &mut ClientBridge,
    kick: &Kick,
    can_stay: bool,
) -> Settled {
    let message = shown_reason(route, kick);
    notify(route, client, kick, message, can_stay).await
}

async fn to_limbo(
    route: &Route<'_>,
    client: &mut ClientBridge,
    kick: &Kick,
    limbo_handlers: Vec<String>,
    can_stay: bool,
) -> Settled {
    let names = if limbo_handlers.is_empty() {
        server_limbo_handlers(route, &kick.server)
    } else {
        limbo_handlers
    };
    let handlers = route
        .services
        .limbo_handler_registry
        .resolve_handlers_lenient(&names);
    if handlers.is_empty() {
        tracing::warn!(server = %kick.server, "SendToLimbo after a kick but no limbo handlers resolved");
        return fall_back(route, client, kick, can_stay).await;
    }
    let entry = LimboEntryContext::KickedFromServer {
        server: kick.server.clone(),
        reason: shown_reason(route, kick),
    };
    Settled::Limbo(handlers, entry, kick.server.clone())
}

async fn disconnect(
    route: &Route<'_>,
    client: &mut ClientBridge,
    kick: &Kick,
    reason: Option<Component>,
) -> ProxyLoopOutcome {
    let registry = &route.services.packet_registry;
    let reported = match (reason, &kick.packet) {
        (Some(reason), _) => {
            client.disconnect(&reason, registry).await.ok();
            Some(reason)
        }
        (None, Some(packet)) if packet.state == client.state() => {
            client.close_with(&packet.frame).await.ok();
            Some(packet.reason.clone())
        }
        (None, Some(packet)) => {
            client.disconnect(&packet.reason, registry).await.ok();
            Some(packet.reason.clone())
        }
        (None, None) => {
            client
                .disconnect(&server_message(route, &kick.server), registry)
                .await
                .ok();
            None
        }
    };
    match &kick.cause {
        KickCause::Unreachable { error } => {
            ProxyLoopOutcome::Error(CoreError::BackendUnreachable(error.clone()))
        }
        _ => ProxyLoopOutcome::BackendClosed { reason: reported },
    }
}

async fn chat(route: &Route<'_>, client: &mut ClientBridge, message: &Component) {
    if let Ok(frame) = crate::player::packets::build_system_chat_message(
        message,
        route.version,
        &route.services.packet_registry,
    ) {
        let _ = client.write_frame(&frame).await;
    }
}

async fn leave_login(route: &Route<'_>, client: &mut ClientBridge) -> Result<(), CoreError> {
    if client.state() == ConnectionState::Login {
        super::auth::complete_login(
            client,
            route.profile,
            route.version,
            &route.services.packet_registry,
        )
        .await?;
    }
    Ok(())
}

async fn enter_play(route: &Route<'_>, client: &mut ClientBridge) -> Result<(), CoreError> {
    leave_login(route, client).await?;
    if client.state() == ConnectionState::Config {
        crate::limbo::login::complete_config_for_limbo(
            client,
            route.version,
            &route.services.packet_registry,
            &route.services.registry_codec_cache,
        )
        .await?;
    }
    Ok(())
}

fn server_limbo_handlers(route: &Route<'_>, server: &ServerId) -> Vec<String> {
    route
        .services
        .domain_router
        .find_by_server_id(server.as_str())
        .map(|config| config.limbo_handlers.clone())
        .unwrap_or_default()
}

fn server_message(route: &Route<'_>, server: &ServerId) -> Component {
    let message = route
        .services
        .domain_router
        .find_by_server_id(server.as_str())
        .map_or_else(
            || UNREACHABLE_MESSAGE.to_string(),
            |config| config.effective_disconnect_message().to_string(),
        );
    Component::text(message)
}

fn shown_reason(route: &Route<'_>, kick: &Kick) -> Component {
    kick.reason()
        .unwrap_or_else(|| server_message(route, &kick.server))
}
