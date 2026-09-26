//! Limbo handler chain — sequential handler execution with Hold support.
//!
//! Runs a chain of [`LimboHandler`] instances sequentially for a player in limbo.
//! Each handler can accept, deny, redirect, or hold. The hold loop processes
//! keepalive, chat, commands, and outgoing frames while waiting for completion.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ConnectionState;

use super::LIMBO_SWITCH_TARGET;
use super::chat::{ClientMessage, parse_client_message};
use super::keepalive::{
    KeepAliveState, KeepAliveTick, extract_keepalive_id, is_keepalive_response,
};
use super::session::LimboSessionImpl;
use super::spawn::send_spawn_sequence;
use super::virtual_session::VirtualSessionCore;
use crate::player::commands::{CommandInbox, CommandOutcome};
use crate::player::packets::build_system_chat_message;
use crate::services::command_manager::DispatchOutcome;
use crate::session::client_bridge::ClientBridge;
use crate::session::context::{SessionContext, SessionIo};
use crate::session::frame_chain::FrameChain;

#[derive(Debug)]
pub(crate) enum LimboChainResult {
    Completed,
    Switch(ServerId),
    Kick(Component),
    ClientDisconnected,
    Shutdown,
    Timeout,
    SendToLimbo(Vec<String>),
}

const KEEPALIVE_INTERVAL_SECS: u64 = 10;

pub(crate) struct Limbo {
    pub(crate) session: Arc<LimboSessionImpl>,
    pub(crate) core: VirtualSessionCore,
    pub(crate) keepalive: KeepAliveState,
    pub(crate) server: ServerId,
}

pub(crate) struct Hold {
    timeout: Option<HoldTimeout>,
    complete: oneshot::Receiver<HandlerResult>,
}

pub(crate) async fn run_handler_chain(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
    limbo: &mut Limbo,
    handlers: &[Arc<dyn LimboHandler>],
    needs_join_game: bool,
) -> LimboChainResult {
    let mut spawn_sent = false;

    for handler in handlers {
        let complete = limbo.session.begin_handler();
        let result = handler.on_player_enter(limbo.session.as_ref()).await;

        match process_handler_result(result) {
            HandlerAction::Continue => continue,
            HandlerAction::Exit(chain_result) => return chain_result,
            HandlerAction::Hold(timeout) => {
                if !spawn_sent {
                    if let Err(e) = send_spawn_sequence(
                        &mut io.client,
                        ctx.version(),
                        ctx.registry(),
                        needs_join_game,
                    )
                    .await
                    {
                        tracing::warn!(error = %e, "failed to send limbo spawn sequence");
                        return LimboChainResult::Kick(Component::text("Internal error"));
                    }
                    spawn_sent = true;
                }
                let hold = Hold { timeout, complete };
                match wait_for_hold(ctx, io, limbo, handler.as_ref(), hold).await {
                    HandlerAction::Continue => continue,
                    HandlerAction::Exit(chain_result) => return chain_result,
                    HandlerAction::Hold(_) => {
                        tracing::warn!(
                            "limbo complete() delivered a Hold result; treating as Accept"
                        );
                        continue;
                    }
                }
            }
        }
    }

    LimboChainResult::Completed
}

#[derive(Debug)]
enum HandlerAction {
    Continue,
    Exit(LimboChainResult),
    Hold(Option<HoldTimeout>),
}

#[derive(Debug)]
struct HoldTimeout {
    after: Duration,
    on_timeout: HandlerResult,
}

fn process_handler_result(result: HandlerResult) -> HandlerAction {
    match result {
        HandlerResult::Accept => HandlerAction::Continue,
        HandlerResult::Deny(reason) => HandlerAction::Exit(LimboChainResult::Kick(reason)),
        HandlerResult::Redirect(server) => HandlerAction::Exit(LimboChainResult::Switch(server)),
        HandlerResult::SendToLimbo(handlers) => {
            HandlerAction::Exit(LimboChainResult::SendToLimbo(handlers))
        }
        HandlerResult::Hold => HandlerAction::Hold(None),
        HandlerResult::HoldWithTimeout { after, on_timeout } => {
            let on_timeout = match *on_timeout {
                HandlerResult::Hold | HandlerResult::HoldWithTimeout { .. } => {
                    HandlerResult::Accept
                }
                other => other,
            };
            HandlerAction::Hold(Some(HoldTimeout { after, on_timeout }))
        }
        // HandlerResult is #[non_exhaustive]; treat unknown variants as Accept.
        _ => HandlerAction::Continue,
    }
}

async fn wait_for_hold(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
    limbo: &mut Limbo,
    handler: &dyn LimboHandler,
    hold: Hold,
) -> HandlerAction {
    let SessionIo {
        client, commands, ..
    } = io;
    let Limbo {
        session,
        core,
        keepalive,
        server,
    } = limbo;
    let Hold {
        timeout,
        mut complete,
    } = hold;
    let services = ctx.services;
    let mut keepalive_interval =
        tokio::time::interval(Duration::from_secs(KEEPALIVE_INTERVAL_SECS));

    let (timeout_after, on_timeout) = match timeout {
        Some(t) => (Some(t.after), Some(t.on_timeout)),
        None => (None, None),
    };
    let hold_timeout = async move {
        match timeout_after {
            Some(after) => tokio::time::sleep(after).await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(hold_timeout);

    let player = &ctx.session;
    let observer = FrameChain::new(ctx, server);

    let released = commands.drain(client, &core.packet_registry, true);
    if let Some(action) = settle_commands(client, commands, &core.packet_registry, released).await {
        return action;
    }

    loop {
        tokio::select! {
            biased;

            Some(command) = commands.recv() => {
                let outcome = commands.apply(command, client, &core.packet_registry, true);
                if let Some(action) = settle_commands(client, commands, &core.packet_registry, outcome).await {
                    return action;
                }
            }

            () = ctx.token.cancelled() => {
                if let Some(reason) = commands.take_kick(client, &core.packet_registry, true) {
                    return HandlerAction::Exit(LimboChainResult::Kick(reason));
                }
                return HandlerAction::Exit(LimboChainResult::Shutdown);
            }

            frame = client.read_frame() => {
                match frame {
                    Ok(Some(frame)) => {
                        observer.observe(&frame, ConnectionState::Play);
                        if is_keepalive_response(&frame, &core.packet_registry, core.protocol_version) {
                            if let Some(id) = extract_keepalive_id(&frame, core.protocol_version) {
                                match keepalive.on_response(id) {
                                    Some(rtt) => player.client_state().record_ping(rtt),
                                    None => tracing::debug!(id, "limbo keepalive response ID mismatch"),
                                }
                            }
                        } else if let Some(msg) = parse_client_message(&frame, &core.packet_registry, core.protocol_version) {
                            match msg {
                                ClientMessage::Command { name, args } => {
                                    let input = if args.is_empty() {
                                        name.clone()
                                    } else {
                                        format!("{name} {}", args.join(" "))
                                    };
                                    let outcome = player.dispatch_command(&services.command_manager, &input);
                                    if outcome == DispatchOutcome::Unknown {
                                        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                                        handler.on_command(session.as_ref(), &name, &args_refs).await;
                                    }
                                }
                                ClientMessage::Chat { message, signed } => {
                                    if let Some(message) = limbo_chat(ctx, client, message, signed).await {
                                        handler.on_chat(session.as_ref(), &message).await;
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) | Err(_) => return HandlerAction::Exit(LimboChainResult::ClientDisconnected),
                }
            }

            frame = core.outgoing_rx.recv() => {
                if let Some(frame) = frame
                    && client.write_frame(&frame).await.is_err() {
                        return HandlerAction::Exit(LimboChainResult::ClientDisconnected);
                    }
            }

            _ = keepalive_interval.tick() => {
                match keepalive.tick(core.protocol_version, &core.packet_registry) {
                    Ok(KeepAliveTick::Send(frame)) => {
                        if client.write_frame(&frame).await.is_err() {
                            return HandlerAction::Exit(LimboChainResult::ClientDisconnected);
                        }
                    }
                    Ok(KeepAliveTick::Idle) => {}
                    Ok(KeepAliveTick::Timeout) | Err(_) => {
                        return HandlerAction::Exit(LimboChainResult::Timeout);
                    }
                }
            }

            result = &mut complete => {
                debug_assert!(
                    result.is_ok(),
                    "limbo hold completion sender dropped without sending a result"
                );
                return match result {
                    Ok(result) => process_handler_result(result),
                    Err(_) => HandlerAction::Continue,
                };
            }

            () = &mut hold_timeout => {
                if let Some(result) = on_timeout.clone() {
                    return process_handler_result(result);
                }
            }
        }
    }
}

async fn limbo_chat(
    ctx: &SessionContext<'_>,
    client: &mut ClientBridge,
    message: String,
    signed: bool,
) -> Option<String> {
    let event = ctx
        .services
        .event_bus
        .fire(ChatMessageEvent::new(ctx.player(), message, signed, None))
        .await;
    match event.result().clone() {
        ChatMessageResult::Deny { reason } => {
            if let Some(reason) = reason {
                match build_system_chat_message(&reason, ctx.version(), ctx.registry()) {
                    Ok(frame) => {
                        if let Err(e) = client.write_frame(&frame).await {
                            tracing::debug!("failed to send a chat denial reason in limbo: {e}");
                        }
                    }
                    Err(e) => tracing::warn!("failed to encode a chat denial reason: {e}"),
                }
            }
            None
        }
        ChatMessageResult::Modify { message } => Some(message),
        _ => Some(event.message),
    }
}

async fn settle_commands(
    client: &mut ClientBridge,
    commands: &mut CommandInbox,
    registry: &PacketRegistry,
    mut outcome: CommandOutcome,
) -> Option<HandlerAction> {
    let exit = loop {
        match outcome {
            CommandOutcome::Switch(target, _) if target.as_str() == LIMBO_SWITCH_TARGET => {
                tracing::debug!("ignoring a request to enter limbo from limbo");
                outcome = commands.drain(client, registry, true);
            }
            CommandOutcome::Switch(target, _) => break Some(LimboChainResult::Switch(target)),
            CommandOutcome::Kick(reason) => break Some(LimboChainResult::Kick(reason)),
            CommandOutcome::Continue => break None,
        }
    };
    commands.deliver_messages(client, None, registry, true);
    if client.flush().await.is_err() {
        return Some(HandlerAction::Exit(LimboChainResult::ClientDisconnected));
    }
    exit.map(HandlerAction::Exit)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use tokio::net::TcpStream;
    use tokio_util::sync::CancellationToken;

    use infrarust_api::event::BoxFuture;
    use infrarust_api::limbo::context::LimboEntryContext;
    use infrarust_api::limbo::handler::HandlerResult;
    use infrarust_api::limbo::session::LimboSession;
    use infrarust_api::types::{Component, PlayerId, ServerId};
    use infrarust_protocol::version::ProtocolVersion;
    use infrarust_transport::BackendConnector;

    use super::super::session::LimboSessionImpl;
    use super::super::test_helpers::*;
    use super::super::virtual_session::VirtualSessionCore;
    use super::*;
    use crate::player::PlayerSession;
    use crate::services::ProxyServices;
    use crate::session::context::test_helpers::{test_connector, test_context, test_io};

    #[test]
    fn test_process_handler_result_accept() {
        assert!(matches!(
            process_handler_result(HandlerResult::Accept),
            HandlerAction::Continue
        ));
    }

    #[test]
    fn test_process_handler_result_deny() {
        let reason = Component::text("go away");
        match process_handler_result(HandlerResult::Deny(reason)) {
            HandlerAction::Exit(LimboChainResult::Kick(r)) => {
                assert_eq!(r, Component::text("go away"));
            }
            other => panic!("expected Exit(Kick), got {other:?}"),
        }
    }

    #[test]
    fn test_process_handler_result_redirect() {
        let server = ServerId::new("lobby");
        match process_handler_result(HandlerResult::Redirect(server)) {
            HandlerAction::Exit(LimboChainResult::Switch(s)) => {
                assert_eq!(s, ServerId::new("lobby"));
            }
            other => panic!("expected Exit(Switch), got {other:?}"),
        }
    }

    #[test]
    fn test_process_handler_result_hold() {
        assert!(matches!(
            process_handler_result(HandlerResult::Hold),
            HandlerAction::Hold(None)
        ));
    }

    #[test]
    fn test_process_handler_result_send_to_limbo() {
        let names = vec!["auth".to_string()];
        match process_handler_result(HandlerResult::SendToLimbo(names)) {
            HandlerAction::Exit(LimboChainResult::SendToLimbo(n)) => {
                assert_eq!(n, vec!["auth".to_string()]);
            }
            other => panic!("expected Exit(SendToLimbo), got {other:?}"),
        }
    }

    fn make_chain_plumbing() -> Limbo {
        let registry = Arc::new(test_registry());
        let player_id = PlayerId::new(1);
        let profile = test_profile();
        let version = ProtocolVersion::V1_21;

        let core = VirtualSessionCore::new(profile.clone(), version, Arc::clone(&registry));

        let session = LimboSessionImpl::new(
            player_id,
            profile,
            version,
            LimboEntryContext::InitialConnection {
                target_server: ServerId::new("test"),
            },
            core.outgoing_tx.clone(),
            CancellationToken::new(),
            registry,
        );

        Limbo {
            session,
            core,
            keepalive: KeepAliveState::new(),
            server: ServerId::new("test"),
        }
    }

    struct Plumbing<'a> {
        ctx: SessionContext<'a>,
        io: SessionIo,
        limbo: Limbo,
        raw: TcpStream,
    }

    async fn plumbing<'a>(
        services: &'a ProxyServices,
        connector: &'a BackendConnector,
    ) -> Plumbing<'a> {
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = test_context(services, connector, player);
        let (client, raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let io = test_io(&ctx, client);
        Plumbing {
            ctx,
            io,
            limbo: make_chain_plumbing(),
            raw,
        }
    }

    fn complete_later(session: &Arc<LimboSessionImpl>, delay_ms: u64, result: HandlerResult) {
        let session = Arc::clone(session);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            session.complete(result);
        });
    }

    async fn run(handlers: Vec<Arc<dyn LimboHandler>>) -> LimboChainResult {
        let services = test_proxy_services();
        let connector = test_connector();
        let mut p = plumbing(&services, &connector).await;
        run_handler_chain(&p.ctx, &mut p.io, &mut p.limbo, &handlers, true).await
    }

    #[tokio::test]
    async fn test_chain_all_accept() {
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![
            Arc::new(FixedHandler {
                name: "h1",
                result: HandlerResult::Accept,
            }),
            Arc::new(FixedHandler {
                name: "h2",
                result: HandlerResult::Accept,
            }),
            Arc::new(FixedHandler {
                name: "h3",
                result: HandlerResult::Accept,
            }),
        ];

        let result = run(handlers).await;
        assert!(matches!(result, LimboChainResult::Completed));
    }

    #[tokio::test]
    async fn test_chain_deny_short_circuits() {
        let second_called = Arc::new(AtomicBool::new(false));
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![
            Arc::new(FixedHandler {
                name: "deny",
                result: HandlerResult::Deny(Component::text("kicked")),
            }),
            Arc::new(TrackingHandler {
                name: "never",
                result: HandlerResult::Accept,
                called: Arc::clone(&second_called),
            }),
        ];

        let result = run(handlers).await;
        assert!(matches!(result, LimboChainResult::Kick(_)));
        assert!(
            !second_called.load(Ordering::SeqCst),
            "second handler should not have been called"
        );
    }

    #[tokio::test]
    async fn test_chain_redirect() {
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![
            Arc::new(FixedHandler {
                name: "accept",
                result: HandlerResult::Accept,
            }),
            Arc::new(FixedHandler {
                name: "redirect",
                result: HandlerResult::Redirect(ServerId::new("lobby")),
            }),
        ];

        let result = run(handlers).await;
        match result {
            LimboChainResult::Switch(s) => assert_eq!(s, ServerId::new("lobby")),
            other => panic!("expected Switch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_chain_hold_then_accept() {
        let services = test_proxy_services();
        let connector = test_connector();
        let mut p = plumbing(&services, &connector).await;
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(HoldHandler { name: "hold" })];

        complete_later(&p.limbo.session, 50, HandlerResult::Accept);

        let result = run_handler_chain(&p.ctx, &mut p.io, &mut p.limbo, &handlers, true).await;
        assert!(matches!(result, LimboChainResult::Completed));
    }

    #[tokio::test]
    async fn test_chain_hold_then_redirect() {
        let services = test_proxy_services();
        let connector = test_connector();
        let mut p = plumbing(&services, &connector).await;
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(HoldHandler { name: "hold" })];

        complete_later(
            &p.limbo.session,
            50,
            HandlerResult::Redirect(ServerId::new("survival")),
        );

        let result = run_handler_chain(&p.ctx, &mut p.io, &mut p.limbo, &handlers, true).await;
        match result {
            LimboChainResult::Switch(s) => assert_eq!(s, ServerId::new("survival")),
            other => panic!("expected Switch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_chain_shutdown_during_hold() {
        let services = test_proxy_services();
        let connector = test_connector();
        let mut p = plumbing(&services, &connector).await;
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(HoldHandler { name: "hold" })];

        let cancel = p.ctx.token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel.cancel();
        });

        let result = run_handler_chain(&p.ctx, &mut p.io, &mut p.limbo, &handlers, true).await;
        assert!(matches!(result, LimboChainResult::Shutdown));
    }

    #[tokio::test]
    async fn test_chain_client_disconnect_during_hold() {
        let services = test_proxy_services();
        let connector = test_connector();
        let Plumbing {
            ctx,
            mut io,
            mut limbo,
            raw,
        } = plumbing(&services, &connector).await;
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(HoldHandler { name: "hold" })];

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(raw);
        });

        let result = run_handler_chain(&ctx, &mut io, &mut limbo, &handlers, true).await;
        assert!(matches!(result, LimboChainResult::ClientDisconnected));
    }

    #[tokio::test]
    async fn test_chain_empty_handlers() {
        let result = run(Vec::new()).await;
        assert!(matches!(result, LimboChainResult::Completed));
    }

    #[tokio::test]
    async fn test_chain_hold_with_timeout_denies() {
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(FixedHandler {
            name: "hold_timeout",
            result: HandlerResult::HoldWithTimeout {
                after: Duration::from_millis(50),
                on_timeout: Box::new(HandlerResult::Deny(Component::text("timed out"))),
            },
        })];

        let result = run(handlers).await;
        assert!(matches!(result, LimboChainResult::Kick(_)));
    }

    #[tokio::test]
    async fn test_hold_with_timeout_complete_wins_over_deadline() {
        let services = test_proxy_services();
        let connector = test_connector();
        let mut p = plumbing(&services, &connector).await;
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(FixedHandler {
            name: "hold_timeout",
            result: HandlerResult::HoldWithTimeout {
                after: Duration::from_secs(30),
                on_timeout: Box::new(HandlerResult::Deny(Component::text("should not fire"))),
            },
        })];

        complete_later(&p.limbo.session, 50, HandlerResult::Accept);

        let result = run_handler_chain(&p.ctx, &mut p.io, &mut p.limbo, &handlers, true).await;
        assert!(matches!(result, LimboChainResult::Completed));
    }

    /// Completes the session with `completion` during `on_player_enter`, then
    /// returns `result` -- leaving the completion latched but unconsumed.
    struct CompleteThenReturn {
        completion: HandlerResult,
        result: HandlerResult,
    }

    impl LimboHandler for CompleteThenReturn {
        fn name(&self) -> &str {
            "complete_then_return"
        }

        fn on_player_enter<'a>(
            &'a self,
            session: &'a dyn LimboSession,
        ) -> BoxFuture<'a, HandlerResult> {
            session.complete(self.completion.clone());
            let result = self.result.clone();
            Box::pin(async move { result })
        }
    }

    #[tokio::test]
    async fn unconsumed_completion_cannot_release_next_handlers_hold() {
        let services = test_proxy_services();
        let connector = test_connector();
        let mut p = plumbing(&services, &connector).await;

        // Handler A latches a Redirect that its Accept return never consumes;
        // handler B's Hold must not be released by it.
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![
            Arc::new(CompleteThenReturn {
                completion: HandlerResult::Redirect(ServerId::new("survival")),
                result: HandlerResult::Accept,
            }),
            Arc::new(HoldHandler { name: "hold" }),
        ];

        complete_later(&p.limbo.session, 150, HandlerResult::Accept);

        let result = run_handler_chain(&p.ctx, &mut p.io, &mut p.limbo, &handlers, true).await;
        assert!(
            matches!(result, LimboChainResult::Completed),
            "handler B must stay held until its own completion, got {result:?}"
        );
    }

    #[tokio::test]
    async fn timeout_race_stale_completion_does_not_release_next_hold() {
        let services = test_proxy_services();
        let connector = test_connector();
        let mut p = plumbing(&services, &connector).await;

        // Handler A held with a timeout; a complete() landed at the deadline
        // but the timeout arm won with a non-terminal result, so the
        // completion was never consumed.
        let _rx_a = p.limbo.session.begin_handler();
        p.limbo
            .session
            .complete(HandlerResult::Redirect(ServerId::new("survival")));

        // Handler B's hold must not see A's stale completion.
        let rx_b = p.limbo.session.begin_handler();
        let handler = HoldHandler { name: "b" };
        let held = tokio::time::timeout(
            Duration::from_millis(200),
            wait_for_hold(
                &p.ctx,
                &mut p.io,
                &mut p.limbo,
                &handler,
                Hold {
                    timeout: None,
                    complete: rx_b,
                },
            ),
        )
        .await;
        assert!(
            held.is_err(),
            "handler B was released by handler A's stale completion: {held:?}"
        );
    }

    #[tokio::test]
    async fn test_hold_with_timeout_nested_hold_coerced_to_accept() {
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(FixedHandler {
            name: "nested",
            result: HandlerResult::HoldWithTimeout {
                after: Duration::from_millis(50),
                on_timeout: Box::new(HandlerResult::Hold),
            },
        })];

        let result = run(handlers).await;
        assert!(matches!(result, LimboChainResult::Completed));
    }
}
