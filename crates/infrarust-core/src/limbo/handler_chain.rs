//! Limbo handler chain — sequential handler execution with Hold support.
//!
//! Runs a chain of [`LimboHandler`] instances sequentially for a player in limbo.
//! Each handler can accept, deny, redirect, or hold. The hold loop processes
//! keepalive, chat, commands, and outgoing frames while waiting for completion.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use super::chat::{ClientMessage, parse_client_message};
use super::keepalive::{
    KeepAliveState, KeepAliveTick, extract_keepalive_id, is_keepalive_response,
};
use super::session::LimboSessionImpl;
use super::spawn::send_spawn_sequence;
use super::virtual_session::VirtualSessionCore;
use crate::services::ProxyServices;
use crate::session::client_bridge::ClientBridge;

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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_handler_chain(
    handlers: &[Arc<dyn LimboHandler>],
    session: Arc<LimboSessionImpl>,
    client: &mut ClientBridge,
    core: &mut VirtualSessionCore,
    keepalive: &mut KeepAliveState,
    services: &ProxyServices,
    cancel: CancellationToken,
    version: ProtocolVersion,
    registry: &PacketRegistry,
    needs_join_game: bool,
) -> LimboChainResult {
    let mut spawn_sent = false;

    for handler in handlers {
        let complete_rx = session.begin_handler();
        let result = handler.on_player_enter(session.as_ref()).await;

        match process_handler_result(result) {
            HandlerAction::Continue => continue,
            HandlerAction::Exit(chain_result) => return chain_result,
            HandlerAction::Hold(timeout) => {
                if !spawn_sent {
                    if let Err(e) =
                        send_spawn_sequence(client, version, registry, needs_join_game).await
                    {
                        tracing::warn!(error = %e, "failed to send limbo spawn sequence");
                        return LimboChainResult::Kick(Component::text("Internal error"));
                    }
                    spawn_sent = true;
                }
                match wait_for_hold(
                    handler.as_ref(),
                    &session,
                    client,
                    core,
                    keepalive,
                    services,
                    cancel.clone(),
                    timeout,
                    complete_rx,
                )
                .await
                {
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

#[allow(clippy::too_many_arguments)]
async fn wait_for_hold(
    handler: &dyn LimboHandler,
    session: &Arc<LimboSessionImpl>,
    client: &mut ClientBridge,
    core: &mut VirtualSessionCore,
    keepalive: &mut KeepAliveState,
    services: &ProxyServices,
    cancel: CancellationToken,
    timeout: Option<HoldTimeout>,
    mut complete_rx: oneshot::Receiver<HandlerResult>,
) -> HandlerAction {
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

    loop {
        tokio::select! {
            frame = client.read_frame() => {
                match frame {
                    Ok(Some(frame)) => {
                        if is_keepalive_response(&frame, &core.packet_registry, core.protocol_version) {
                            if let Some(id) = extract_keepalive_id(&frame, core.protocol_version)
                                && !keepalive.on_response(id) {
                                    tracing::debug!(id, "limbo keepalive response ID mismatch");
                                }
                        } else if let Some(msg) = parse_client_message(&frame, &core.packet_registry, core.protocol_version) {
                            match msg {
                                ClientMessage::Command { name, args } => {
                                    let input = if args.is_empty() {
                                        name.clone()
                                    } else {
                                        format!("{name} {}", args.join(" "))
                                    };
                                    let handled = services.command_manager.dispatch(
                                        Some(core.player_id),
                                        &input,
                                        services.player_registry.as_ref(),
                                    ).await;
                                    if !handled {
                                        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                                        handler.on_command(session.as_ref(), &name, &args_refs).await;
                                    }
                                }
                                ClientMessage::Chat { message } => {
                                    handler.on_chat(session.as_ref(), &message).await;
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

            result = &mut complete_rx => {
                debug_assert!(
                    result.is_ok(),
                    "limbo hold completion sender dropped without sending a result"
                );
                return match result {
                    Ok(result) => process_handler_result(result),
                    Err(_) => HandlerAction::Continue,
                };
            }

            () = cancel.cancelled() => {
                return HandlerAction::Exit(LimboChainResult::Shutdown);
            }

            () = &mut hold_timeout => {
                if let Some(result) = on_timeout.clone() {
                    return process_handler_result(result);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use tokio_util::sync::CancellationToken;

    use infrarust_api::event::BoxFuture;
    use infrarust_api::limbo::context::LimboEntryContext;
    use infrarust_api::limbo::handler::HandlerResult;
    use infrarust_api::limbo::session::LimboSession;
    use infrarust_api::types::{Component, PlayerId, ServerId};
    use infrarust_protocol::version::ProtocolVersion;

    use super::super::session::LimboSessionImpl;
    use super::super::test_helpers::*;
    use super::super::virtual_session::VirtualSessionCore;
    use super::*;

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
                assert_eq!(r.to_json(), Component::text("go away").to_json());
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

    fn make_chain_plumbing() -> (
        Arc<LimboSessionImpl>,
        VirtualSessionCore,
        KeepAliveState,
        Arc<PacketRegistry>,
    ) {
        let registry = Arc::new(test_registry());
        let player_id = PlayerId::new(1);
        let profile = test_profile();
        let version = ProtocolVersion::V1_21;

        let core =
            VirtualSessionCore::new(player_id, profile.clone(), version, Arc::clone(&registry));

        let session = LimboSessionImpl::new(
            player_id,
            profile,
            version,
            LimboEntryContext::InitialConnection {
                target_server: ServerId::new("test"),
            },
            core.outgoing_tx.clone(),
            CancellationToken::new(),
            Arc::clone(&registry),
        );

        (Arc::new(session), core, KeepAliveState::new(), registry)
    }

    fn complete_later(session: &Arc<LimboSessionImpl>, delay_ms: u64, result: HandlerResult) {
        let session = Arc::clone(session);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            session.complete(result);
        });
    }

    #[tokio::test]
    async fn test_chain_all_accept() {
        let (session, mut core, mut keepalive, registry) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

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

        let result = run_handler_chain(
            &handlers,
            session,
            &mut client,
            &mut core,
            &mut keepalive,
            &services,
            cancel,
            ProtocolVersion::V1_21,
            &registry,
            true,
        )
        .await;
        assert!(matches!(result, LimboChainResult::Completed));
    }

    #[tokio::test]
    async fn test_chain_deny_short_circuits() {
        let (session, mut core, mut keepalive, registry) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

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

        let result = run_handler_chain(
            &handlers,
            session,
            &mut client,
            &mut core,
            &mut keepalive,
            &services,
            cancel,
            ProtocolVersion::V1_21,
            &registry,
            true,
        )
        .await;
        assert!(matches!(result, LimboChainResult::Kick(_)));
        assert!(
            !second_called.load(Ordering::SeqCst),
            "second handler should not have been called"
        );
    }

    #[tokio::test]
    async fn test_chain_redirect() {
        let (session, mut core, mut keepalive, registry) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

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

        let result = run_handler_chain(
            &handlers,
            session,
            &mut client,
            &mut core,
            &mut keepalive,
            &services,
            cancel,
            ProtocolVersion::V1_21,
            &registry,
            true,
        )
        .await;
        match result {
            LimboChainResult::Switch(s) => assert_eq!(s, ServerId::new("lobby")),
            other => panic!("expected Switch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_chain_hold_then_accept() {
        let (session, mut core, mut keepalive, registry) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(HoldHandler { name: "hold" })];

        complete_later(&session, 50, HandlerResult::Accept);

        let result = run_handler_chain(
            &handlers,
            session,
            &mut client,
            &mut core,
            &mut keepalive,
            &services,
            cancel,
            ProtocolVersion::V1_21,
            &registry,
            true,
        )
        .await;
        assert!(matches!(result, LimboChainResult::Completed));
    }

    #[tokio::test]
    async fn test_chain_hold_then_redirect() {
        let (session, mut core, mut keepalive, registry) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(HoldHandler { name: "hold" })];

        complete_later(
            &session,
            50,
            HandlerResult::Redirect(ServerId::new("survival")),
        );

        let result = run_handler_chain(
            &handlers,
            session,
            &mut client,
            &mut core,
            &mut keepalive,
            &services,
            cancel,
            ProtocolVersion::V1_21,
            &registry,
            true,
        )
        .await;
        match result {
            LimboChainResult::Switch(s) => assert_eq!(s, ServerId::new("survival")),
            other => panic!("expected Switch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_chain_shutdown_during_hold() {
        let (session, mut core, mut keepalive, registry) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(HoldHandler { name: "hold" })];

        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_clone.cancel();
        });

        let result = run_handler_chain(
            &handlers,
            session,
            &mut client,
            &mut core,
            &mut keepalive,
            &services,
            cancel,
            ProtocolVersion::V1_21,
            &registry,
            true,
        )
        .await;
        assert!(matches!(result, LimboChainResult::Shutdown));
    }

    #[tokio::test]
    async fn test_chain_client_disconnect_during_hold() {
        let (session, mut core, mut keepalive, registry) = make_chain_plumbing();
        let (mut client, raw_stream) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(HoldHandler { name: "hold" })];

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(raw_stream);
        });

        let result = run_handler_chain(
            &handlers,
            session,
            &mut client,
            &mut core,
            &mut keepalive,
            &services,
            cancel,
            ProtocolVersion::V1_21,
            &registry,
            true,
        )
        .await;
        assert!(matches!(result, LimboChainResult::ClientDisconnected));
    }

    #[tokio::test]
    async fn test_chain_empty_handlers() {
        let (session, mut core, mut keepalive, registry) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![];

        let result = run_handler_chain(
            &handlers,
            session,
            &mut client,
            &mut core,
            &mut keepalive,
            &services,
            cancel,
            ProtocolVersion::V1_21,
            &registry,
            true,
        )
        .await;
        assert!(matches!(result, LimboChainResult::Completed));
    }

    #[tokio::test]
    async fn test_chain_hold_with_timeout_denies() {
        let (session, mut core, mut keepalive, registry) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(FixedHandler {
            name: "hold_timeout",
            result: HandlerResult::HoldWithTimeout {
                after: Duration::from_millis(50),
                on_timeout: Box::new(HandlerResult::Deny(Component::text("timed out"))),
            },
        })];

        let result = run_handler_chain(
            &handlers,
            session,
            &mut client,
            &mut core,
            &mut keepalive,
            &services,
            cancel,
            ProtocolVersion::V1_21,
            &registry,
            true,
        )
        .await;
        assert!(matches!(result, LimboChainResult::Kick(_)));
    }

    #[tokio::test]
    async fn test_hold_with_timeout_complete_wins_over_deadline() {
        let (session, mut core, mut keepalive, registry) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(FixedHandler {
            name: "hold_timeout",
            result: HandlerResult::HoldWithTimeout {
                after: Duration::from_secs(30),
                on_timeout: Box::new(HandlerResult::Deny(Component::text("should not fire"))),
            },
        })];

        complete_later(&session, 50, HandlerResult::Accept);

        let result = run_handler_chain(
            &handlers,
            session,
            &mut client,
            &mut core,
            &mut keepalive,
            &services,
            cancel,
            ProtocolVersion::V1_21,
            &registry,
            true,
        )
        .await;
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
        let (session, mut core, mut keepalive, registry) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        // Handler A latches a Redirect that its Accept return never consumes;
        // handler B's Hold must not be released by it.
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![
            Arc::new(CompleteThenReturn {
                completion: HandlerResult::Redirect(ServerId::new("survival")),
                result: HandlerResult::Accept,
            }),
            Arc::new(HoldHandler { name: "hold" }),
        ];

        complete_later(&session, 150, HandlerResult::Accept);

        let result = run_handler_chain(
            &handlers,
            session,
            &mut client,
            &mut core,
            &mut keepalive,
            &services,
            cancel,
            ProtocolVersion::V1_21,
            &registry,
            true,
        )
        .await;
        assert!(
            matches!(result, LimboChainResult::Completed),
            "handler B must stay held until its own completion, got {result:?}"
        );
    }

    #[tokio::test]
    async fn timeout_race_stale_completion_does_not_release_next_hold() {
        let (session, mut core, mut keepalive, _registry) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();

        // Handler A held with a timeout; a complete() landed at the deadline
        // but the timeout arm won with a non-terminal result, so the
        // completion was never consumed.
        let _rx_a = session.begin_handler();
        session.complete(HandlerResult::Redirect(ServerId::new("survival")));

        // Handler B's hold must not see A's stale completion.
        let rx_b = session.begin_handler();
        let handler = HoldHandler { name: "b" };
        let held = tokio::time::timeout(
            Duration::from_millis(200),
            wait_for_hold(
                &handler,
                &session,
                &mut client,
                &mut core,
                &mut keepalive,
                &services,
                CancellationToken::new(),
                None,
                rx_b,
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
        let (session, mut core, mut keepalive, registry) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(FixedHandler {
            name: "nested",
            result: HandlerResult::HoldWithTimeout {
                after: Duration::from_millis(50),
                on_timeout: Box::new(HandlerResult::Hold),
            },
        })];

        let result = run_handler_chain(
            &handlers,
            session,
            &mut client,
            &mut core,
            &mut keepalive,
            &services,
            cancel,
            ProtocolVersion::V1_21,
            &registry,
            true,
        )
        .await;
        assert!(matches!(result, LimboChainResult::Completed));
    }
}
