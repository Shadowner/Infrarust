//! Limbo handler chain — sequential handler execution with Hold support.
//!
//! Runs a chain of [`LimboHandler`] instances sequentially for a player in limbo.
//! Each handler can accept, deny, redirect, or hold. The hold loop processes
//! keepalive, chat, commands, and outgoing frames while waiting for completion.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::types::{Component, ServerId};

use super::chat::{ClientMessage, parse_client_message};
use super::keepalive::{KeepAliveState, extract_keepalive_id, is_keepalive_response};
use super::session::LimboSessionImpl;
use super::virtual_session::VirtualSessionCore;
use crate::services::ProxyServices;
use crate::session::client_bridge::ClientBridge;

pub(crate) struct LimboLoopState {
    pub complete_rx: watch::Receiver<Option<HandlerResult>>,
    pub keepalive: KeepAliveState,
}

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

pub(crate) async fn run_handler_chain(
    handlers: &[Arc<dyn LimboHandler>],
    session: Arc<LimboSessionImpl>,
    client: &mut ClientBridge,
    core: &mut VirtualSessionCore,
    limbo_state: &mut LimboLoopState,
    services: &ProxyServices,
    cancel: CancellationToken,
) -> LimboChainResult {
    for handler in handlers {
        let result = handler.on_player_enter(session.as_ref()).await;

        match process_handler_result(result) {
            HandlerAction::Continue => continue,
            HandlerAction::Exit(chain_result) => return chain_result,
            HandlerAction::Hold => {
                match wait_for_hold(
                    handler.as_ref(),
                    &session,
                    client,
                    core,
                    limbo_state,
                    services,
                    cancel.clone(),
                )
                .await
                {
                    HandlerAction::Continue => continue,
                    HandlerAction::Exit(chain_result) => return chain_result,
                    HandlerAction::Hold => unreachable!("complete() cannot return Hold"),
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
    Hold,
}

fn process_handler_result(result: HandlerResult) -> HandlerAction {
    match result {
        HandlerResult::Accept => HandlerAction::Continue,
        HandlerResult::Deny(reason) => HandlerAction::Exit(LimboChainResult::Kick(reason)),
        HandlerResult::Redirect(server) => HandlerAction::Exit(LimboChainResult::Switch(server)),
        HandlerResult::SendToLimbo(handlers) => HandlerAction::Exit(LimboChainResult::SendToLimbo(handlers)),
        HandlerResult::Hold => HandlerAction::Hold,
        // HandlerResult is #[non_exhaustive]; treat unknown variants as Accept.
        _ => HandlerAction::Continue,
    }
}

async fn wait_for_hold(
    handler: &dyn LimboHandler,
    session: &Arc<LimboSessionImpl>,
    client: &mut ClientBridge,
    core: &mut VirtualSessionCore,
    limbo_state: &mut LimboLoopState,
    services: &ProxyServices,
    cancel: CancellationToken,
) -> HandlerAction {
    let mut keepalive_interval =
        tokio::time::interval(Duration::from_secs(KEEPALIVE_INTERVAL_SECS));

    loop {
        tokio::select! {
            frame = client.read_frame() => {
                match frame {
                    Ok(Some(frame)) => {
                        if is_keepalive_response(&frame, &core.packet_registry, core.protocol_version) {
                            if let Some(id) = extract_keepalive_id(&frame, core.protocol_version) {
                                limbo_state.keepalive.on_response(id);
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
                if let Some(frame) = frame {
                    if client.write_frame(&frame).await.is_err() {
                        return HandlerAction::Exit(LimboChainResult::ClientDisconnected);
                    }
                }
            }

            _ = keepalive_interval.tick() => {
                match limbo_state.keepalive.tick(core.protocol_version, &core.packet_registry) {
                    Ok(Some(frame)) => {
                        if client.write_frame(&frame).await.is_err() {
                            return HandlerAction::Exit(LimboChainResult::ClientDisconnected);
                        }
                    }
                    Ok(None) | Err(_) => return HandlerAction::Exit(LimboChainResult::Timeout),
                }
            }

            _ = limbo_state.complete_rx.changed() => {
                if let Some(result) = limbo_state.complete_rx.borrow_and_update().clone() {
                    return process_handler_result(result);
                }
            }

            () = cancel.cancelled() => {
                return HandlerAction::Exit(LimboChainResult::Shutdown);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use tokio::sync::watch;
    use tokio_util::sync::CancellationToken;

    use infrarust_api::limbo::context::LimboEntryContext;
    use infrarust_api::limbo::handler::HandlerResult;
    use infrarust_api::types::{Component, PlayerId, ServerId};
    use infrarust_protocol::version::ProtocolVersion;

    use super::*;
    use super::super::keepalive::KeepAliveState;
    use super::super::session::LimboSessionImpl;
    use super::super::virtual_session::VirtualSessionCore;
    use super::super::test_helpers::*;

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
            HandlerAction::Hold
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
        LimboLoopState,
        watch::Sender<Option<HandlerResult>>,
    ) {
        let registry = Arc::new(test_registry());
        let player_id = PlayerId::new(1);
        let profile = test_profile();
        let version = ProtocolVersion::V1_21;

        let core = VirtualSessionCore::new(player_id, profile.clone(), version, Arc::clone(&registry));
        let (complete_tx, complete_rx) = watch::channel::<Option<HandlerResult>>(None);

        let session = LimboSessionImpl::new(
            player_id,
            profile,
            version,
            LimboEntryContext::InitialConnection,
            core.outgoing_tx.clone(),
            complete_tx.clone(),
            Arc::clone(&registry),
        );

        let limbo_state = LimboLoopState {
            complete_rx,
            keepalive: KeepAliveState::new(),
        };

        (Arc::new(session), core, limbo_state, complete_tx)
    }

    #[tokio::test]
    async fn test_chain_all_accept() {
        let (session, mut core, mut limbo_state, _complete_tx) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![
            Arc::new(FixedHandler { name: "h1", result: HandlerResult::Accept }),
            Arc::new(FixedHandler { name: "h2", result: HandlerResult::Accept }),
            Arc::new(FixedHandler { name: "h3", result: HandlerResult::Accept }),
        ];

        let result = run_handler_chain(&handlers, session, &mut client, &mut core, &mut limbo_state, &services, cancel).await;
        assert!(matches!(result, LimboChainResult::Completed));
    }

    #[tokio::test]
    async fn test_chain_deny_short_circuits() {
        let (session, mut core, mut limbo_state, _complete_tx) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let second_called = Arc::new(AtomicBool::new(false));
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![
            Arc::new(FixedHandler { name: "deny", result: HandlerResult::Deny(Component::text("kicked")) }),
            Arc::new(TrackingHandler { name: "never", result: HandlerResult::Accept, called: Arc::clone(&second_called) }),
        ];

        let result = run_handler_chain(&handlers, session, &mut client, &mut core, &mut limbo_state, &services, cancel).await;
        assert!(matches!(result, LimboChainResult::Kick(_)));
        assert!(!second_called.load(Ordering::SeqCst), "second handler should not have been called");
    }

    #[tokio::test]
    async fn test_chain_redirect() {
        let (session, mut core, mut limbo_state, _complete_tx) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![
            Arc::new(FixedHandler { name: "accept", result: HandlerResult::Accept }),
            Arc::new(FixedHandler { name: "redirect", result: HandlerResult::Redirect(ServerId::new("lobby")) }),
        ];

        let result = run_handler_chain(&handlers, session, &mut client, &mut core, &mut limbo_state, &services, cancel).await;
        match result {
            LimboChainResult::Switch(s) => assert_eq!(s, ServerId::new("lobby")),
            other => panic!("expected Switch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_chain_hold_then_accept() {
        let (session, mut core, mut limbo_state, complete_tx) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![
            Arc::new(HoldHandler { name: "hold" }),
        ];

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            complete_tx.send(Some(HandlerResult::Accept)).unwrap();
        });

        let result = run_handler_chain(&handlers, session, &mut client, &mut core, &mut limbo_state, &services, cancel).await;
        assert!(matches!(result, LimboChainResult::Completed));
    }

    #[tokio::test]
    async fn test_chain_hold_then_redirect() {
        let (session, mut core, mut limbo_state, complete_tx) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![
            Arc::new(HoldHandler { name: "hold" }),
        ];

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            complete_tx.send(Some(HandlerResult::Redirect(ServerId::new("survival")))).unwrap();
        });

        let result = run_handler_chain(&handlers, session, &mut client, &mut core, &mut limbo_state, &services, cancel).await;
        match result {
            LimboChainResult::Switch(s) => assert_eq!(s, ServerId::new("survival")),
            other => panic!("expected Switch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_chain_shutdown_during_hold() {
        let (session, mut core, mut limbo_state, _complete_tx) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![
            Arc::new(HoldHandler { name: "hold" }),
        ];

        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            cancel_clone.cancel();
        });

        let result = run_handler_chain(&handlers, session, &mut client, &mut core, &mut limbo_state, &services, cancel).await;
        assert!(matches!(result, LimboChainResult::Shutdown));
    }

    #[tokio::test]
    async fn test_chain_client_disconnect_during_hold() {
        let (session, mut core, mut limbo_state, _complete_tx) = make_chain_plumbing();
        let (mut client, raw_stream) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![
            Arc::new(HoldHandler { name: "hold" }),
        ];

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            drop(raw_stream);
        });

        let result = run_handler_chain(&handlers, session, &mut client, &mut core, &mut limbo_state, &services, cancel).await;
        assert!(matches!(result, LimboChainResult::ClientDisconnected));
    }

    #[tokio::test]
    async fn test_chain_empty_handlers() {
        let (session, mut core, mut limbo_state, _complete_tx) = make_chain_plumbing();
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let cancel = CancellationToken::new();

        let handlers: Vec<Arc<dyn LimboHandler>> = vec![];

        let result = run_handler_chain(&handlers, session, &mut client, &mut core, &mut limbo_state, &services, cancel).await;
        assert!(matches!(result, LimboChainResult::Completed));
    }
}
