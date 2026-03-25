//! Limbo engine -- `enter_limbo()` orchestrator.
//!
//! Coordinates the full lifecycle of a player in the limbo world:
//! spawn sequence, session setup, handler chain execution, and cleanup.

use std::sync::Arc;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::types::{Component, GameProfile, PlayerId, ServerId};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use super::handler_chain::{LimboChainResult, LimboLoopState, run_handler_chain};
use super::keepalive::KeepAliveState;
use super::session::LimboSessionImpl;
use super::virtual_session::VirtualSessionCore;
use crate::player::packets::build_disconnect;
use crate::services::ProxyServices;
use crate::session::client_bridge::ClientBridge;

#[derive(Debug)]
pub(crate) enum LimboExitResult {
    Completed,
    SwitchedTo(ServerId),
    /// Disconnect packet already sent.
    Kicked,
    ClientDisconnected,
    Shutdown,
    Timeout,
    SendToLimbo(Vec<String>),
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn enter_limbo(
    client: &mut ClientBridge,
    handlers: Vec<Arc<dyn LimboHandler>>,
    player_id: PlayerId,
    profile: GameProfile,
    version: ProtocolVersion,
    entry_context: LimboEntryContext,
    registry: &PacketRegistry,
    services: &ProxyServices,
    cancel: CancellationToken,
) -> LimboExitResult {
    let mut core = VirtualSessionCore::new(
        player_id,
        profile,
        version,
        Arc::clone(&services.packet_registry),
    );

    let (complete_tx, complete_rx) = watch::channel::<Option<HandlerResult>>(None);
    let session = LimboSessionImpl::new(
        player_id,
        core.profile.clone(),
        version,
        entry_context,
        core.outgoing_tx.clone(),
        complete_tx,
        Arc::clone(&services.packet_registry),
    );

    let mut limbo_state = LimboLoopState {
        complete_rx,
        keepalive: KeepAliveState::new(),
    };

    let session = Arc::new(session);
    session.set_self_ref(Arc::downgrade(&session));

    let chain_result = run_handler_chain(
        &handlers,
        session,
        client,
        &mut core,
        &mut limbo_state,
        services,
        cancel,
        version,
        registry,
        true,
    )
    .await;

    map_chain_result(
        chain_result,
        client,
        version,
        registry,
        &handlers,
        player_id,
    )
    .await
}

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

        LimboChainResult::SendToLimbo(handler_names) => LimboExitResult::SendToLimbo(handler_names),
    }
}

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

async fn fire_on_disconnect(handlers: &[Arc<dyn LimboHandler>], player_id: PlayerId) {
    for handler in handlers {
        handler.on_disconnect(player_id).await;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;

    use infrarust_api::limbo::handler::HandlerResult;
    use infrarust_api::types::{Component, PlayerId, ServerId};
    use infrarust_protocol::version::ProtocolVersion;

    use super::super::handler_chain::LimboChainResult;
    use super::super::test_helpers::*;
    use super::*;

    #[tokio::test]
    async fn test_map_completed() {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let registry = Arc::new(test_registry());
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![];
        let result = map_chain_result(
            LimboChainResult::Completed,
            &mut client,
            ProtocolVersion::V1_21,
            &registry,
            &handlers,
            PlayerId::new(1),
        )
        .await;
        assert!(matches!(result, LimboExitResult::Completed));
    }

    #[tokio::test]
    async fn test_map_switch() {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let registry = Arc::new(test_registry());
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![];
        let result = map_chain_result(
            LimboChainResult::Switch(ServerId::new("lobby")),
            &mut client,
            ProtocolVersion::V1_21,
            &registry,
            &handlers,
            PlayerId::new(1),
        )
        .await;
        match result {
            LimboExitResult::SwitchedTo(s) => assert_eq!(s, ServerId::new("lobby")),
            other => panic!("expected SwitchedTo, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_map_kick() {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let registry = Arc::new(test_registry());
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![];
        let result = map_chain_result(
            LimboChainResult::Kick(Component::text("bye")),
            &mut client,
            ProtocolVersion::V1_21,
            &registry,
            &handlers,
            PlayerId::new(1),
        )
        .await;
        assert!(matches!(result, LimboExitResult::Kicked));
    }

    #[tokio::test]
    async fn test_map_client_disconnected() {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let registry = Arc::new(test_registry());
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(FixedHandler {
            name: "h1",
            result: HandlerResult::Accept,
        })];
        let result = map_chain_result(
            LimboChainResult::ClientDisconnected,
            &mut client,
            ProtocolVersion::V1_21,
            &registry,
            &handlers,
            PlayerId::new(1),
        )
        .await;
        assert!(matches!(result, LimboExitResult::ClientDisconnected));
    }

    #[tokio::test]
    async fn test_map_shutdown() {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let registry = Arc::new(test_registry());
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![];
        let result = map_chain_result(
            LimboChainResult::Shutdown,
            &mut client,
            ProtocolVersion::V1_21,
            &registry,
            &handlers,
            PlayerId::new(1),
        )
        .await;
        assert!(matches!(result, LimboExitResult::Shutdown));
    }

    #[tokio::test]
    async fn test_map_timeout() {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let registry = Arc::new(test_registry());
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![];
        let result = map_chain_result(
            LimboChainResult::Timeout,
            &mut client,
            ProtocolVersion::V1_21,
            &registry,
            &handlers,
            PlayerId::new(1),
        )
        .await;
        assert!(matches!(result, LimboExitResult::Timeout));
    }

    #[tokio::test]
    async fn test_map_send_to_limbo() {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let registry = Arc::new(test_registry());
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![];
        let names = vec!["auth".to_string(), "lobby".to_string()];
        let result = map_chain_result(
            LimboChainResult::SendToLimbo(names),
            &mut client,
            ProtocolVersion::V1_21,
            &registry,
            &handlers,
            PlayerId::new(1),
        )
        .await;
        match result {
            LimboExitResult::SendToLimbo(n) => assert_eq!(n, vec!["auth", "lobby"]),
            other => panic!("expected SendToLimbo, got {other:?}"),
        }
    }
}
