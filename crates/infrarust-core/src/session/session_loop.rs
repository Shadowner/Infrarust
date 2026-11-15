//! Unified session loop for intercepted proxy modes.

use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::{
    ConnectCause, KickCause, KickedFromServerEvent, KickedFromServerResult,
};
use infrarust_api::events::limbo::{LimboEnterEvent, LimboExitEvent, LimboExitReason};
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::{HANDLER_UNAVAILABLE, LimboHandler};
use infrarust_api::player::ConnectionResult;
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::version::ConnectionState;

use crate::error::CoreError;
use crate::limbo::LIMBO_SWITCH_TARGET;
use crate::limbo::engine::{LimboExitResult, enter_limbo};
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;
use crate::session::client_login::complete_login;
use crate::session::context::{SessionContext, SessionIo};
use crate::session::frame_chain::FrameChain;
use crate::session::kick::{Kick, MAX_KICK_REDIRECTS};
use crate::session::proxy_loop::{ProxyLoopOutcome, proxy_loop};
use crate::session::server_join::ServerJoin;
use crate::session::server_switch::{SwitchResult, SwitchTarget};

const UNREACHABLE_MESSAGE: &str = "Server is currently unreachable. Please try again later.";

pub(crate) async fn run_session_loop(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
    initial: Initial,
) -> ProxyLoopOutcome {
    let Initial {
        mut mode,
        server,
        pending,
    } = initial;
    let mut session = SessionLoop {
        ctx,
        io,
        server,
        pending,
    };

    loop {
        if ctx.token.is_cancelled() && !matches!(mode, ConnectionMode::Backend(_)) {
            break ProxyLoopOutcome::Shutdown;
        }
        let transition = match &mut mode {
            ConnectionMode::Backend(backend) => session.serve_backend(backend).await,
            ConnectionMode::Kicked(kick) => session.after_kick(kick.clone()).await,
            ConnectionMode::Limbo(entry) => session.hold_in_limbo(entry).await,
        };
        match transition {
            Transition::Stay => {}
            Transition::Backend(backend, server) => {
                mode = ConnectionMode::Backend(backend);
                session.server = server;
                session.pending.join = None;
            }
            Transition::Limbo(entry, server) => {
                mode = ConnectionMode::Limbo(entry);
                session.server = server;
            }
            Transition::Kicked(kick) => mode = ConnectionMode::Kicked(kick),
            Transition::End(outcome) => break outcome,
        }
    }
}

enum Transition {
    Stay,
    Backend(BackendBridge, ServerId),
    Limbo(LimboEntry, ServerId),
    Kicked(Kick),
    End(ProxyLoopOutcome),
}

struct SessionLoop<'a, 'c> {
    ctx: &'a SessionContext<'c>,
    io: &'a mut SessionIo,
    server: ServerId,
    pending: Pending,
}

impl SessionLoop<'_, '_> {
    async fn serve_backend(&mut self, backend: &mut BackendBridge) -> Transition {
        let session = &self.ctx.session;
        session.set_connected_address(backend.server_address().cloned());
        self.pending.approved = None;
        let outcome = proxy_loop(
            self.ctx,
            self.io,
            backend,
            &self.server,
            &mut self.pending.join,
        )
        .await;

        match outcome {
            ProxyLoopOutcome::SwitchRequested { target, .. }
                if target.as_str() == LIMBO_SWITCH_TARGET =>
            {
                self.redirect_to_limbo(&target).await
            }
            ProxyLoopOutcome::SwitchRequested { target, .. } if target == self.server => {
                tracing::debug!(server = %target, "already on the requested server");
                session.settle_connect(&target, &ConnectionResult::AlreadyConnected);
                Transition::Stay
            }
            ProxyLoopOutcome::SwitchRequested { target, cause } => {
                self.switch_server(target, cause).await
            }
            ProxyLoopOutcome::BackendKick(packet) => {
                let during_connect = self.pending.join.take().is_some();
                Transition::Kicked(Kick::from_packet(
                    self.server.clone(),
                    *packet,
                    during_connect,
                ))
            }
            ProxyLoopOutcome::BackendDisconnected { reason } => {
                tracing::debug!(?reason, "backend connection lost");
                let during_connect = self.pending.join.take().is_some();
                Transition::Kicked(Kick::lost(self.server.clone(), during_connect))
            }
            other => Transition::End(other),
        }
    }

    async fn redirect_to_limbo(&mut self, target: &ServerId) -> Transition {
        let services = self.ctx.services;
        let handler_names = server_limbo_handlers(self.ctx, &self.server);
        let reason = match services
            .limbo_handler_registry
            .resolve_handlers(&handler_names)
        {
            Ok(handlers) if !handlers.is_empty() => {
                self.ctx
                    .session
                    .settle_connect(target, &ConnectionResult::Success);
                let entry = LimboEntry {
                    handlers,
                    context: LimboEntryContext::PluginRedirect {
                        from_server: Some(self.server.clone()),
                    },
                };
                return Transition::Limbo(entry, self.server.clone());
            }
            Ok(_) => {
                tracing::warn!("no limbo handlers configured, disconnecting");
                Component::text("No limbo handlers configured for this server")
            }
            Err(e) => {
                tracing::warn!(error = %e, "limbo handler not loaded, disconnecting");
                Component::text(HANDLER_UNAVAILABLE)
            }
        };
        self.kick(reason).await
    }

    async fn switch_server(&mut self, target: ServerId, cause: ConnectCause) -> Transition {
        let ctx = self.ctx;
        let request = SwitchTarget::Unapproved {
            server: target.clone(),
            cause,
        };
        let action = switch(ctx, self.io, &self.server, request).await;
        ctx.session
            .settle_connect(&target, &connection_result(ctx, &action));
        match action {
            SwitchResult::Backend(backend, server) => Transition::Backend(backend, server),
            SwitchResult::Unchanged => Transition::Stay,
            SwitchResult::Limbo(entry) if entry.handlers.is_empty() => {
                tracing::warn!(
                    "SendToLimbo during switch but no handlers, staying on current server"
                );
                Transition::Stay
            }
            SwitchResult::Limbo(entry) => Transition::Limbo(entry, self.server.clone()),
            SwitchResult::Denied(reason) => {
                tracing::info!(reason = %reason, "server switch denied by event");
                chat(ctx, &mut self.io.client, &reason).await;
                Transition::Stay
            }
            SwitchResult::Failed(kick) => settle(ctx, self.io, &self.server, kick, true).await,
            SwitchResult::Error(e) => {
                tracing::warn!("server switch failed: {e}");
                let message = Component::text(format!("Server switch failed: {e}"));
                chat(ctx, &mut self.io.client, &message).await;
                Transition::Stay
            }
        }
    }

    async fn after_kick(&mut self, kick: Kick) -> Transition {
        self.ctx.session.set_connected_address(None);
        self.pending.join = None;
        match settle(self.ctx, self.io, &self.server, kick, false).await {
            Transition::Stay => Transition::End(ProxyLoopOutcome::Error(CoreError::InvalidState(
                "a kicked player has no server to stay on",
            ))),
            transition => transition,
        }
    }

    async fn hold_in_limbo(&mut self, entry: &LimboEntry) -> Transition {
        let ctx = self.ctx;
        let services = ctx.services;
        ctx.session.set_connected_address(None);
        self.pending.join = None;
        if let Err(e) = enter_play(ctx, self.io, &self.server).await {
            tracing::warn!("could not bring the client into play for limbo: {e}");
            self.io
                .client
                .disconnect(&Component::text(e.to_string()), &services.packet_registry)
                .await
                .ok();
            return Transition::End(ProxyLoopOutcome::Error(e));
        }
        let player = ctx.player();
        services
            .event_bus
            .fire(LimboEnterEvent::new(
                Arc::clone(&player),
                entry
                    .handlers
                    .iter()
                    .map(|h| h.name().to_string())
                    .collect(),
                entry.context.clone(),
            ))
            .await;
        let exit = enter_limbo(
            ctx,
            self.io,
            entry.handlers.clone(),
            entry.context.clone(),
            &self.server,
        )
        .await;

        let shutting_down = ctx.token.is_cancelled();
        let (reason, next_server) = limbo_exit(&exit, shutting_down, &self.server);
        services
            .event_bus
            .fire(LimboExitEvent::new(player, reason, next_server))
            .await;

        Transition::End(match exit {
            LimboExitResult::Completed | LimboExitResult::SwitchedTo(_) if shutting_down => {
                ProxyLoopOutcome::Shutdown
            }
            LimboExitResult::Completed => {
                return self.leave_limbo(self.server.clone(), entry).await;
            }
            LimboExitResult::SwitchedTo(target) => return self.leave_limbo(target, entry).await,
            LimboExitResult::SendToLimbo(handler_names) => {
                return self.move_to_limbo(&handler_names);
            }
            LimboExitResult::Kicked(reason) => ProxyLoopOutcome::Kicked { reason },
            LimboExitResult::ClientDisconnected => ProxyLoopOutcome::ClientDisconnected,
            LimboExitResult::Timeout => ProxyLoopOutcome::Error(CoreError::Timeout(
                "the client stopped answering limbo keepalives".into(),
            )),
            LimboExitResult::Error(e) => ProxyLoopOutcome::Error(e),
            LimboExitResult::Shutdown => ProxyLoopOutcome::Shutdown,
        })
    }

    async fn leave_limbo(&mut self, target: ServerId, entry: &LimboEntry) -> Transition {
        let ctx = self.ctx;
        let gate_target = match &entry.context {
            LimboEntryContext::InitialConnection { target_server } => Some(target_server),
            _ => None,
        };
        let from_initial = gate_target.is_some();
        let requested = target.clone();
        let target = self.pending.release(target, gate_target);
        let action = switch(ctx, self.io, &self.server, target).await;
        ctx.session
            .settle_connect(&requested, &connection_result(ctx, &action));
        match action {
            SwitchResult::Backend(backend, server) => Transition::Backend(backend, server),
            SwitchResult::Unchanged => Transition::End(ProxyLoopOutcome::Error(
                CoreError::InvalidState("a limbo exit kept no server to join"),
            )),
            SwitchResult::Limbo(next) => {
                if from_initial || next.handlers.is_empty() {
                    if from_initial {
                        tracing::warn!(
                            "skipping re-entry into limbo after initial connection gate"
                        );
                    }
                    return Transition::End(ProxyLoopOutcome::Error(CoreError::InvalidState(
                        "no limbo handlers left to continue with",
                    )));
                }
                Transition::Limbo(next, self.server.clone())
            }
            SwitchResult::Denied(reason) => {
                tracing::info!(reason = %reason, "switch after limbo denied by event");
                self.kick(reason).await
            }
            SwitchResult::Failed(kick) => Transition::Kicked(kick),
            SwitchResult::Error(e) => {
                tracing::warn!("switch after limbo failed: {e}");
                Transition::End(ProxyLoopOutcome::Error(e))
            }
        }
    }

    fn move_to_limbo(&self, handler_names: &[String]) -> Transition {
        let handlers = match self
            .ctx
            .services
            .limbo_handler_registry
            .resolve_handlers(handler_names)
        {
            Ok(handlers) => handlers,
            Err(e) => {
                tracing::warn!(error = %e, "limbo-to-limbo move refused");
                return Transition::End(ProxyLoopOutcome::Error(e));
            }
        };
        if handlers.is_empty() {
            tracing::warn!("limbo-to-limbo but no valid handlers resolved, disconnecting");
            return Transition::End(ProxyLoopOutcome::Error(CoreError::InvalidState(
                "no limbo handlers resolved for a limbo-to-limbo move",
            )));
        }
        let entry = LimboEntry {
            handlers,
            context: LimboEntryContext::PluginRedirect {
                from_server: Some(self.server.clone()),
            },
        };
        Transition::Limbo(entry, self.server.clone())
    }

    async fn kick(&mut self, reason: Component) -> Transition {
        self.io
            .client
            .disconnect(&reason, self.ctx.registry())
            .await
            .ok();
        Transition::End(ProxyLoopOutcome::Kicked { reason })
    }
}

fn limbo_exit(
    exit: &LimboExitResult,
    shutting_down: bool,
    held_for: &ServerId,
) -> (LimboExitReason, Option<ServerId>) {
    match exit {
        LimboExitResult::Completed | LimboExitResult::SwitchedTo(_) if shutting_down => {
            (LimboExitReason::Shutdown, None)
        }
        LimboExitResult::Completed => (LimboExitReason::Released, Some(held_for.clone())),
        LimboExitResult::SwitchedTo(server) => (LimboExitReason::Redirected, Some(server.clone())),
        LimboExitResult::SendToLimbo(handlers) => (
            LimboExitReason::SentToLimbo {
                handlers: handlers.clone(),
            },
            None,
        ),
        LimboExitResult::Kicked(reason) => (
            LimboExitReason::Kicked {
                reason: reason.clone(),
            },
            None,
        ),
        LimboExitResult::ClientDisconnected | LimboExitResult::Error(_) => {
            (LimboExitReason::Disconnected, None)
        }
        LimboExitResult::Timeout => (LimboExitReason::TimedOut, None),
        LimboExitResult::Shutdown => (LimboExitReason::Shutdown, None),
    }
}

pub(crate) enum ConnectionMode {
    Backend(BackendBridge),
    Limbo(LimboEntry),
    Kicked(Kick),
}

pub(crate) struct LimboEntry {
    pub(crate) handlers: Vec<Arc<dyn LimboHandler>>,
    pub(crate) context: LimboEntryContext,
}

pub(crate) struct Initial {
    pub(crate) mode: ConnectionMode,
    pub(crate) server: ServerId,
    pub(crate) pending: Pending,
}

pub(crate) struct Pending {
    join: Option<ServerJoin>,
    approved: Option<ServerId>,
}

impl Pending {
    pub(crate) const fn join(join: ServerJoin) -> Self {
        Self {
            join: Some(join),
            approved: None,
        }
    }

    pub(crate) const fn approved(server: ServerId) -> Self {
        Self {
            join: None,
            approved: Some(server),
        }
    }

    pub(crate) const fn nothing() -> Self {
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

async fn switch(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
    current_server: &ServerId,
    target: SwitchTarget,
) -> SwitchResult {
    crate::session::server_switch::perform_switch(ctx, io, current_server, target)
        .await
        .unwrap_or_else(SwitchResult::Error)
}

fn connection_result(ctx: &SessionContext<'_>, action: &SwitchResult) -> ConnectionResult {
    if ctx.session.shutdown_token().is_cancelled() {
        return ConnectionResult::Cancelled;
    }
    match action {
        SwitchResult::Backend(..) => ConnectionResult::Success,
        SwitchResult::Unchanged => ConnectionResult::AlreadyConnected,
        SwitchResult::Limbo(..) => ConnectionResult::Cancelled,
        SwitchResult::Denied(reason) => ConnectionResult::Denied(reason.clone()),
        SwitchResult::Failed(kick) => ConnectionResult::Failed(shown_reason(ctx, kick)),
        SwitchResult::Error(e) => ConnectionResult::Failed(Component::text(e.to_string())),
    }
}

async fn settle(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
    current_server: &ServerId,
    mut kick: Kick,
    mut can_stay: bool,
) -> Transition {
    let mut redirects = 0;
    loop {
        if ctx.session.shutdown_token().is_cancelled() {
            return Transition::End(ProxyLoopOutcome::Shutdown);
        }
        can_stay &= !kick.stranded;
        match fire_kicked(ctx, &kick, can_stay).await {
            KickedFromServerResult::Redirect(target) if redirects < MAX_KICK_REDIRECTS => {
                redirects += 1;
                if let Err(e) = leave_login(ctx, io).await {
                    return Transition::End(ProxyLoopOutcome::Error(e));
                }
                let request = SwitchTarget::Unapproved {
                    server: target,
                    cause: ConnectCause::KickRedirect,
                };
                match switch(ctx, io, current_server, request).await {
                    SwitchResult::Backend(backend, server) => {
                        return Transition::Backend(backend, server);
                    }
                    SwitchResult::Limbo(entry) if !entry.handlers.is_empty() => {
                        return Transition::Limbo(entry, current_server.clone());
                    }
                    SwitchResult::Failed(next) => kick = next,
                    SwitchResult::Denied(reason) => {
                        return notify(ctx, &mut io.client, &kick, reason, can_stay).await;
                    }
                    SwitchResult::Limbo(..) | SwitchResult::Unchanged => {
                        return fall_back(ctx, &mut io.client, &kick, can_stay).await;
                    }
                    SwitchResult::Error(e) => {
                        tracing::warn!(server = %kick.server, "kick redirect failed: {e}");
                        return fall_back(ctx, &mut io.client, &kick, can_stay).await;
                    }
                }
            }
            KickedFromServerResult::Redirect(target) => {
                tracing::warn!(
                    server = %kick.server,
                    target = %target,
                    "giving up after {MAX_KICK_REDIRECTS} kick redirects"
                );
                return Transition::End(disconnect(ctx, &mut io.client, &kick, None).await);
            }
            KickedFromServerResult::SendToLimbo { limbo_handlers } => {
                return to_limbo(ctx, &mut io.client, &kick, limbo_handlers, can_stay).await;
            }
            KickedFromServerResult::Notify { message } => {
                return notify(ctx, &mut io.client, &kick, message, can_stay).await;
            }
            KickedFromServerResult::DisconnectPlayer { reason } => {
                return Transition::End(disconnect(ctx, &mut io.client, &kick, reason).await);
            }
            _ => return Transition::End(disconnect(ctx, &mut io.client, &kick, None).await),
        }
    }
}

async fn fire_kicked(
    ctx: &SessionContext<'_>,
    kick: &Kick,
    can_stay: bool,
) -> KickedFromServerResult {
    let player = ctx.player();
    let previous_server = if kick.during_connect {
        player.current_server()
    } else {
        ctx.session.previous_server()
    };
    let event = KickedFromServerEvent::new(
        player,
        kick.server.clone(),
        kick.reason(),
        kick.cause.clone(),
        kick.during_connect,
        previous_server,
        default_result(ctx, kick, can_stay),
    );
    ctx.services.event_bus.fire(event).await.result().clone()
}

fn default_result(ctx: &SessionContext<'_>, kick: &Kick, can_stay: bool) -> KickedFromServerResult {
    if !kick.during_connect {
        return KickedFromServerResult::DisconnectPlayer { reason: None };
    }
    if can_stay {
        return KickedFromServerResult::Notify {
            message: shown_reason(ctx, kick),
        };
    }
    let limbo_handlers = server_limbo_handlers(ctx, &kick.server);
    let resolvable = !limbo_handlers.is_empty()
        && ctx
            .services
            .limbo_handler_registry
            .resolve_handlers(&limbo_handlers)
            .is_ok();
    if resolvable {
        KickedFromServerResult::SendToLimbo { limbo_handlers }
    } else {
        KickedFromServerResult::DisconnectPlayer { reason: None }
    }
}

async fn notify(
    ctx: &SessionContext<'_>,
    client: &mut ClientBridge,
    kick: &Kick,
    message: Component,
    can_stay: bool,
) -> Transition {
    if can_stay {
        chat(ctx, client, &message).await;
        return Transition::Stay;
    }
    Transition::End(disconnect(ctx, client, kick, Some(message)).await)
}

async fn fall_back(
    ctx: &SessionContext<'_>,
    client: &mut ClientBridge,
    kick: &Kick,
    can_stay: bool,
) -> Transition {
    let message = shown_reason(ctx, kick);
    notify(ctx, client, kick, message, can_stay).await
}

async fn to_limbo(
    ctx: &SessionContext<'_>,
    client: &mut ClientBridge,
    kick: &Kick,
    limbo_handlers: Vec<String>,
    can_stay: bool,
) -> Transition {
    let names = if limbo_handlers.is_empty() {
        server_limbo_handlers(ctx, &kick.server)
    } else {
        limbo_handlers
    };
    let handlers = match ctx.services.limbo_handler_registry.resolve_handlers(&names) {
        Ok(handlers) if !handlers.is_empty() => handlers,
        Ok(_) => {
            tracing::warn!(server = %kick.server, "SendToLimbo after a kick but no limbo handlers configured");
            return fall_back(ctx, client, kick, can_stay).await;
        }
        Err(e) => {
            tracing::warn!(server = %kick.server, error = %e, "SendToLimbo after a kick refused");
            return fall_back(ctx, client, kick, can_stay).await;
        }
    };
    let entry = LimboEntry {
        handlers,
        context: LimboEntryContext::KickedFromServer {
            server: kick.server.clone(),
            reason: shown_reason(ctx, kick),
        },
    };
    Transition::Limbo(entry, kick.server.clone())
}

async fn disconnect(
    ctx: &SessionContext<'_>,
    client: &mut ClientBridge,
    kick: &Kick,
    reason: Option<Component>,
) -> ProxyLoopOutcome {
    let registry = ctx.registry();
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
                .disconnect(&shown_reason(ctx, kick), registry)
                .await
                .ok();
            kick.reason()
        }
    };
    match &kick.cause {
        KickCause::Unreachable { error } => {
            ProxyLoopOutcome::Error(CoreError::BackendUnreachable(error.clone()))
        }
        _ => ProxyLoopOutcome::BackendClosed { reason: reported },
    }
}

async fn chat(ctx: &SessionContext<'_>, client: &mut ClientBridge, message: &Component) {
    if let Ok(frame) =
        crate::player::packets::build_system_chat_message(message, ctx.version(), ctx.registry())
    {
        let _ = client.write_frame(&frame).await;
    }
}

async fn leave_login(ctx: &SessionContext<'_>, io: &mut SessionIo) -> Result<(), CoreError> {
    if io.client.state() == ConnectionState::Login {
        complete_login(ctx, io).await?;
    }
    Ok(())
}

async fn enter_play(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
    server: &ServerId,
) -> Result<(), CoreError> {
    leave_login(ctx, io).await?;
    let client = &mut io.client;
    if client.state() == ConnectionState::Config {
        let observer = FrameChain::new(ctx, server);
        crate::limbo::login::complete_config_for_limbo(
            client,
            ctx.version(),
            ctx.registry(),
            &ctx.services.registry_codec_cache,
            Some(&observer),
        )
        .await?;
    }
    Ok(())
}

fn server_limbo_handlers(ctx: &SessionContext<'_>, server: &ServerId) -> Vec<String> {
    ctx.services
        .domain_router
        .find_by_server_id(server.as_str())
        .map(|config| config.limbo_handlers.clone())
        .unwrap_or_default()
}

fn server_message(ctx: &SessionContext<'_>, server: &ServerId) -> Component {
    let message = ctx
        .services
        .domain_router
        .find_by_server_id(server.as_str())
        .map_or_else(
            || UNREACHABLE_MESSAGE.to_string(),
            |config| config.effective_disconnect_message().to_string(),
        );
    Component::text(message)
}

fn shown_reason(ctx: &SessionContext<'_>, kick: &Kick) -> Component {
    kick.reason()
        .unwrap_or_else(|| server_message(ctx, &kick.server))
}
