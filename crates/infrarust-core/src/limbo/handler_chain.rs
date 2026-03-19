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

/// Limbo-specific loop state. Not shared with future Tier 3.
pub(crate) struct LimboLoopState {
    pub complete_rx: watch::Receiver<Option<HandlerResult>>,
    pub keepalive: KeepAliveState,
}

/// Result of running the handler chain — carries data for the engine to act on.
pub(crate) enum LimboChainResult {
    /// All handlers returned Accept.
    Completed,
    /// A handler returned Redirect(server).
    Switch(ServerId),
    /// A handler returned Deny(reason).
    Kick(Component),
    /// Client disconnected during the chain.
    ClientDisconnected,
    /// Proxy shutdown.
    Shutdown,
    /// KeepAlive timeout.
    Timeout,
}

/// KeepAlive interval in the limbo loop.
const KEEPALIVE_INTERVAL_SECS: u64 = 10;

/// Runs the handler chain sequentially.
///
/// For each handler:
/// - Accept -> continue to next
/// - Deny(reason) -> return Kick
/// - Redirect(server) -> return Switch
/// - Hold -> enter wait_for_hold loop
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

/// Internal action from processing a HandlerResult.
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
        HandlerResult::Hold => HandlerAction::Hold,
        // HandlerResult is #[non_exhaustive]; treat unknown variants as Accept.
        _ => HandlerAction::Continue,
    }
}

/// Waits for a handler that returned Hold.
///
/// Runs the select! loop processing keepalive, chat/commands, outgoing frames,
/// and completion signals until the handler calls `session.complete()`.
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
            // 1. Client packets
            frame = client.read_frame() => {
                match frame {
                    Ok(Some(frame)) => {
                        // KeepAlive response?
                        if is_keepalive_response(&frame, &core.packet_registry, core.protocol_version) {
                            if let Some(id) = extract_keepalive_id(&frame, core.protocol_version) {
                                limbo_state.keepalive.on_response(id);
                            }
                        }
                        // Chat / command?
                        else if let Some(msg) = parse_client_message(&frame, &core.packet_registry, core.protocol_version) {
                            match msg {
                                ClientMessage::Command { name, args } => {
                                    // CommandManager first
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
                        // Everything else: silently dropped
                    }
                    Ok(None) => return HandlerAction::Exit(LimboChainResult::ClientDisconnected),
                    Err(_) => return HandlerAction::Exit(LimboChainResult::ClientDisconnected),
                }
            }

            // 2. Outgoing frames from session
            frame = core.outgoing_rx.recv() => {
                if let Some(frame) = frame {
                    if client.write_frame(&frame).await.is_err() {
                        return HandlerAction::Exit(LimboChainResult::ClientDisconnected);
                    }
                }
            }

            // 3. KeepAlive tick
            _ = keepalive_interval.tick() => {
                match limbo_state.keepalive.tick(core.protocol_version, &core.packet_registry) {
                    Ok(Some(frame)) => {
                        if client.write_frame(&frame).await.is_err() {
                            return HandlerAction::Exit(LimboChainResult::ClientDisconnected);
                        }
                    }
                    Ok(None) => return HandlerAction::Exit(LimboChainResult::Timeout),
                    Err(_) => return HandlerAction::Exit(LimboChainResult::Timeout),
                }
            }

            // 4. Hold completion
            _ = limbo_state.complete_rx.changed() => {
                if let Some(result) = limbo_state.complete_rx.borrow_and_update().clone() {
                    return process_handler_result(result);
                }
            }

            // 5. Shutdown
            () = cancel.cancelled() => {
                return HandlerAction::Exit(LimboChainResult::Shutdown);
            }
        }
    }
}
