//! Limbo engine -- `enter_limbo()` orchestrator.
//!
//! Coordinates the full lifecycle of a player in the limbo world:
//! spawn sequence, session setup, handler chain execution, and cleanup.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::{LimboHandler, SessionEndReason};
use infrarust_api::types::{Component, GameProfile, PlayerId, ServerId};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use super::handler_chain::{LimboChainResult, run_handler_chain};
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

    let limbo_token = cancel.child_token();
    let session = LimboSessionImpl::new(
        player_id,
        core.profile.clone(),
        version,
        entry_context,
        core.outgoing_tx.clone(),
        limbo_token.clone(),
        Arc::clone(&services.packet_registry),
    );
    let _limbo_guard = limbo_token.drop_guard();

    let mut keepalive = KeepAliveState::new();

    let session = Arc::new(session);
    session.set_self_ref(Arc::downgrade(&session));

    let chain_result = run_handler_chain(
        &handlers,
        session,
        client,
        &mut core,
        &mut keepalive,
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
    let end_reason = match &result {
        LimboChainResult::Completed => Some(SessionEndReason::Released),
        LimboChainResult::Switch(_) => Some(SessionEndReason::Redirected),
        LimboChainResult::Kick(_) => Some(SessionEndReason::Kicked),
        LimboChainResult::ClientDisconnected => Some(SessionEndReason::Disconnected),
        LimboChainResult::Shutdown => Some(SessionEndReason::Shutdown),
        LimboChainResult::Timeout => Some(SessionEndReason::TimedOut),
        LimboChainResult::SendToLimbo(_) => None,
    };
    if let Some(reason) = end_reason {
        fire_on_session_end(handlers, player_id, reason).await;
    }

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

async fn fire_on_session_end(
    handlers: &[Arc<dyn LimboHandler>],
    player_id: PlayerId,
    reason: SessionEndReason,
) {
    for handler in handlers {
        handler.on_session_end(player_id, reason.clone()).await;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::{Arc, Mutex};

    use infrarust_api::limbo::handler::{HandlerResult, SessionEndReason};
    use infrarust_api::types::{Component, PlayerId, ServerId};
    use infrarust_protocol::version::ProtocolVersion;
    use tokio_util::sync::CancellationToken;

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

    async fn end_reasons_for(result: LimboChainResult) -> Vec<SessionEndReason> {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let registry = Arc::new(test_registry());
        let ended = Arc::new(Mutex::new(Vec::new()));
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(SessionEndRecorder {
            name: "rec",
            result: HandlerResult::Accept,
            ended: Arc::clone(&ended),
        })];
        let _ = map_chain_result(
            result,
            &mut client,
            ProtocolVersion::V1_21,
            &registry,
            &handlers,
            PlayerId::new(1),
        )
        .await;
        ended.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn on_session_end_fires_for_every_terminal_arm() {
        assert_eq!(
            end_reasons_for(LimboChainResult::Completed).await,
            vec![SessionEndReason::Released]
        );
        assert_eq!(
            end_reasons_for(LimboChainResult::Switch(ServerId::new("lobby"))).await,
            vec![SessionEndReason::Redirected]
        );
        assert_eq!(
            end_reasons_for(LimboChainResult::Kick(Component::text("bye"))).await,
            vec![SessionEndReason::Kicked]
        );
        assert_eq!(
            end_reasons_for(LimboChainResult::ClientDisconnected).await,
            vec![SessionEndReason::Disconnected]
        );
        assert_eq!(
            end_reasons_for(LimboChainResult::Shutdown).await,
            vec![SessionEndReason::Shutdown]
        );
        assert_eq!(
            end_reasons_for(LimboChainResult::Timeout).await,
            vec![SessionEndReason::TimedOut]
        );
    }

    #[tokio::test]
    async fn send_to_limbo_is_a_continuation_not_a_session_end() {
        assert!(
            end_reasons_for(LimboChainResult::SendToLimbo(vec!["auth".to_string()]))
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn per_session_token_cancelled_after_enter_limbo_returns() {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let services = test_proxy_services();
        let captured = Arc::new(Mutex::new(None));
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(TokenCaptureHandler {
            name: "cap",
            result: HandlerResult::Accept,
            captured: Arc::clone(&captured),
        })];

        let exit = enter_limbo(
            &mut client,
            handlers,
            PlayerId::new(1),
            test_profile(),
            ProtocolVersion::V1_21,
            LimboEntryContext::InitialConnection {
                target_server: ServerId::new("lobby"),
            },
            &services.packet_registry,
            &services,
            CancellationToken::new(),
        )
        .await;

        assert!(matches!(exit, LimboExitResult::Completed));
        let token = captured
            .lock()
            .unwrap()
            .clone()
            .expect("handler should have captured the token");
        assert!(
            token.is_cancelled(),
            "per-session token must be cancelled once the limbo session ends"
        );
    }
}
