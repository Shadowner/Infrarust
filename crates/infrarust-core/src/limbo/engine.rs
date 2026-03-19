//! Limbo engine -- `enter_limbo()` orchestrator.
//!
//! Coordinates the full lifecycle of a player in the limbo world:
//! spawn sequence, session setup, handler chain execution, and cleanup.

use std::sync::Arc;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::types::{Component, GameProfile, PlayerId, ServerId};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use super::handler_chain::{LimboChainResult, LimboLoopState, run_handler_chain};
use super::keepalive::KeepAliveState;
use super::session::LimboSessionImpl;
use super::spawn::send_spawn_sequence;
use super::virtual_session::VirtualSessionCore;
use crate::player::packets::build_disconnect;
use crate::services::ProxyServices;
use crate::session::client_bridge::ClientBridge;

/// Clean status returned after the limbo engine finishes.
///
/// The caller (connection handler) uses this to decide the next step:
/// reconnect to a backend, disconnect the client, or clean up.
pub(crate) enum LimboExitResult {
    /// All handlers returned Accept -- caller switches to the original target.
    Completed,
    /// A handler returned Redirect -- caller switches to the specified server.
    SwitchedTo(ServerId),
    /// A handler returned Deny -- disconnect packet already sent.
    Kicked,
    /// The client disconnected during limbo.
    ClientDisconnected,
    /// The proxy is shutting down.
    Shutdown,
    /// KeepAlive timed out -- the client stopped responding.
    Timeout,
}

/// Enters the limbo world for a player.
///
/// This is the main entry point for the limbo engine. It:
/// 1. Optionally sends the spawn sequence (JoinGame, chunks, etc.).
/// 2. Creates the virtual session and handler chain plumbing.
/// 3. Runs the handler chain to completion.
/// 4. Maps the chain result to a clean exit status.
///
/// # Errors
/// Spawn-sequence failures are logged and mapped to [`LimboExitResult::Kicked`]
/// (the client cannot render the world, so staying in limbo is pointless).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn enter_limbo(
    client: &mut ClientBridge,
    handlers: Vec<Arc<dyn LimboHandler>>,
    player_id: PlayerId,
    profile: GameProfile,
    version: ProtocolVersion,
    registry: &PacketRegistry,
    services: &ProxyServices,
    cancel: CancellationToken,
) -> LimboExitResult {
    // 1. Always send the spawn sequence with JoinGame.
    //    The JoinGame packet resets the client's world state (unloads all
    //    chunks and entities). For 1.20.2+ this is sufficient; pre-1.20.2
    //    would also need the Respawn trick (handled inside send_spawn_sequence).
    if let Err(e) = send_spawn_sequence(client, version, registry, true).await {
        warn!(player = %profile.username, error = %e, "failed to send limbo spawn sequence");
        return LimboExitResult::Kicked;
    }

    // 2. Create the virtual session core (identity + outgoing channel).
    let mut core = VirtualSessionCore::new(
        player_id,
        profile,
        version,
        Arc::clone(&services.packet_registry),
    );

    // 3. Create the completion watch channel.
    let (complete_tx, complete_rx) = watch::channel::<Option<HandlerResult>>(None);

    // 4. Build the LimboSessionImpl (bridges API trait to packet encoding).
    let session = LimboSessionImpl::new(
        player_id,
        core.profile.clone(),
        version,
        core.outgoing_tx.clone(),
        complete_tx,
        Arc::clone(&services.packet_registry),
    );

    // 5. Build the loop state.
    let mut limbo_state = LimboLoopState {
        complete_rx,
        keepalive: KeepAliveState::new(),
    };

    // 6. Run the handler chain.
    let chain_result = run_handler_chain(
        &handlers,
        Arc::new(session),
        client,
        &mut core,
        &mut limbo_state,
        services,
        cancel,
    )
    .await;

    // 7. Map chain result to exit result.
    map_chain_result(chain_result, client, version, registry, &handlers, player_id).await
}

/// Maps a [`LimboChainResult`] to a [`LimboExitResult`], performing any
/// necessary side effects (sending disconnect, firing on_disconnect).
async fn map_chain_result(
    result: LimboChainResult,
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
    handlers: &[Arc<dyn LimboHandler>],
    player_id: PlayerId,
) -> LimboExitResult {
    match result {
        LimboChainResult::Completed => LimboExitResult::Completed,

        LimboChainResult::Switch(server_id) => LimboExitResult::SwitchedTo(server_id),

        LimboChainResult::Kick(reason) => {
            send_disconnect(client, &reason, version, registry).await;
            LimboExitResult::Kicked
        }

        LimboChainResult::ClientDisconnected => {
            fire_on_disconnect(handlers, player_id).await;
            LimboExitResult::ClientDisconnected
        }

        LimboChainResult::Shutdown => LimboExitResult::Shutdown,

        LimboChainResult::Timeout => LimboExitResult::Timeout,
    }
}

/// Sends a play-state disconnect packet to the client.
///
/// Errors are silently ignored -- the client is about to be dropped anyway.
async fn send_disconnect(
    client: &mut ClientBridge,
    reason: &Component,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) {
    if let Ok(frame) = build_disconnect(reason, version, registry) {
        let _ = client.write_frame(&frame).await;
    }
}

/// Fires `on_disconnect` for all handlers in the chain.
async fn fire_on_disconnect(handlers: &[Arc<dyn LimboHandler>], player_id: PlayerId) {
    for handler in handlers {
        handler.on_disconnect(player_id).await;
    }
}
