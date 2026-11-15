//! Limbo engine -- `enter_limbo()` orchestrator.
//!
//! Coordinates the full lifecycle of a player in the limbo world:
//! spawn sequence, session setup, handler chain execution, and cleanup.

use std::sync::Arc;

use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::{LimboHandler, SessionEndReason};
use infrarust_api::types::{Component, PlayerId, ServerId};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use super::handler_chain::{Limbo, LimboExit, guarded, run_handler_chain};
use super::keepalive::KeepAliveState;
use super::session::LimboSessionImpl;
use super::virtual_session::VirtualSessionCore;
use crate::player::packets::build_disconnect;
use crate::session::client_bridge::ClientBridge;
use crate::session::context::{SessionContext, SessionIo};

pub(crate) async fn enter_limbo(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
    handlers: Vec<Arc<dyn LimboHandler>>,
    entry_context: LimboEntryContext,
    server: &ServerId,
) -> LimboExit {
    let registry = Arc::clone(&ctx.services.packet_registry);
    let player_id = ctx.player_id();
    let version = ctx.version();
    let core = VirtualSessionCore::new(ctx.profile().clone(), version, Arc::clone(&registry));

    let limbo_token = ctx.token.child_token();
    let session = LimboSessionImpl::new(
        player_id,
        core.profile.clone(),
        version,
        entry_context,
        core.outgoing_tx.clone(),
        limbo_token.clone(),
        Arc::clone(&registry),
    );
    let _limbo_guard = limbo_token.drop_guard();

    let mut limbo = Limbo {
        session,
        core,
        keepalive: KeepAliveState::new(),
        server: server.clone(),
    };

    let exit = run_handler_chain(ctx, io, &mut limbo, &handlers, true).await;
    conclude(
        &exit,
        &mut io.client,
        version,
        &registry,
        &handlers,
        player_id,
    )
    .await;
    exit
}

const fn end_reason(exit: &LimboExit) -> Option<SessionEndReason> {
    match exit {
        LimboExit::Completed => Some(SessionEndReason::Released),
        LimboExit::Switch(_) => Some(SessionEndReason::Redirected),
        LimboExit::Kick(_) => Some(SessionEndReason::Kicked),
        LimboExit::ClientDisconnected | LimboExit::Error(_) => Some(SessionEndReason::Disconnected),
        LimboExit::Shutdown => Some(SessionEndReason::Shutdown),
        LimboExit::Timeout => Some(SessionEndReason::TimedOut),
        LimboExit::SendToLimbo(_) => None,
    }
}

async fn conclude(
    exit: &LimboExit,
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
    handlers: &[Arc<dyn LimboHandler>],
    player_id: PlayerId,
) {
    if let Some(reason) = end_reason(exit) {
        fire_on_session_end(handlers, player_id, reason).await;
    }
    match exit {
        LimboExit::Kick(reason) => send_disconnect(client, reason, version, registry).await,
        LimboExit::ClientDisconnected | LimboExit::Error(_) => {
            fire_on_disconnect(handlers, player_id).await;
        }
        LimboExit::Completed
        | LimboExit::Switch(_)
        | LimboExit::Shutdown
        | LimboExit::Timeout
        | LimboExit::SendToLimbo(_) => {}
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
        guarded(handler.as_ref(), "on_disconnect", || {
            handler.on_disconnect(player_id)
        })
        .await;
    }
}

async fn fire_on_session_end(
    handlers: &[Arc<dyn LimboHandler>],
    player_id: PlayerId,
    reason: SessionEndReason,
) {
    for handler in handlers {
        guarded(handler.as_ref(), "on_session_end", || {
            handler.on_session_end(player_id, reason)
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::{Arc, Mutex};

    use infrarust_api::event::BoxFuture;
    use infrarust_api::limbo::handler::{HandlerResult, SessionEndReason};
    use infrarust_api::limbo::session::LimboSession;
    use infrarust_api::types::{Component, PlayerId, ServerId};
    use infrarust_protocol::version::ProtocolVersion;

    use super::super::test_helpers::*;
    use super::*;
    use crate::player::PlayerSession;
    use crate::session::context::test_helpers::{test_connector, test_context, test_io};

    async fn end_reasons_for(result: LimboExit) -> Vec<SessionEndReason> {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let registry = Arc::new(test_registry());
        let ended = Arc::new(Mutex::new(Vec::new()));
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(SessionEndRecorder {
            name: "rec",
            result: HandlerResult::Accept,
            ended: Arc::clone(&ended),
        })];
        conclude(
            &result,
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
            end_reasons_for(LimboExit::Completed).await,
            vec![SessionEndReason::Released]
        );
        assert_eq!(
            end_reasons_for(LimboExit::Switch(ServerId::new("lobby"))).await,
            vec![SessionEndReason::Redirected]
        );
        assert_eq!(
            end_reasons_for(LimboExit::Kick(Component::text("bye"))).await,
            vec![SessionEndReason::Kicked]
        );
        assert_eq!(
            end_reasons_for(LimboExit::ClientDisconnected).await,
            vec![SessionEndReason::Disconnected]
        );
        assert_eq!(
            end_reasons_for(LimboExit::Shutdown).await,
            vec![SessionEndReason::Shutdown]
        );
        assert_eq!(
            end_reasons_for(LimboExit::Timeout).await,
            vec![SessionEndReason::TimedOut]
        );
    }

    enum Teardown {
        Panics,
        Hangs,
    }

    struct BrokenTeardown(Teardown);

    impl BrokenTeardown {
        fn call(&self) -> BoxFuture<'_, ()> {
            match self.0 {
                Teardown::Panics => panic!("teardown panicked"),
                Teardown::Hangs => Box::pin(std::future::pending()),
            }
        }
    }

    impl LimboHandler for BrokenTeardown {
        fn name(&self) -> &str {
            "broken"
        }

        fn on_player_enter<'a>(
            &'a self,
            _session: &'a dyn LimboSession,
        ) -> BoxFuture<'a, HandlerResult> {
            Box::pin(async { HandlerResult::Accept })
        }

        fn on_disconnect(&self, _player_id: PlayerId) -> BoxFuture<'_, ()> {
            self.call()
        }

        fn on_session_end(
            &self,
            _player_id: PlayerId,
            _reason: SessionEndReason,
        ) -> BoxFuture<'_, ()> {
            self.call()
        }
    }

    async fn end_reasons_after(broken: Teardown) -> Vec<SessionEndReason> {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let registry = Arc::new(test_registry());
        let ended = Arc::new(Mutex::new(Vec::new()));
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![
            Arc::new(BrokenTeardown(broken)),
            Arc::new(SessionEndRecorder {
                name: "rec",
                result: HandlerResult::Accept,
                ended: Arc::clone(&ended),
            }),
        ];
        conclude(
            &LimboExit::ClientDisconnected,
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
    async fn a_teardown_callback_that_panics_does_not_unwind_the_session() {
        assert_eq!(
            end_reasons_after(Teardown::Panics).await,
            vec![SessionEndReason::Disconnected]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_teardown_callback_that_hangs_does_not_block_the_session() {
        assert_eq!(
            end_reasons_after(Teardown::Hangs).await,
            vec![SessionEndReason::Disconnected]
        );
    }

    #[tokio::test]
    async fn send_to_limbo_is_a_continuation_not_a_session_end() {
        assert!(
            end_reasons_for(LimboExit::SendToLimbo(vec!["auth".to_string()]))
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn per_session_token_cancelled_after_enter_limbo_returns() {
        let services = test_proxy_services();
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = test_context(&services, &connector, player);
        let (client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let mut io = test_io(&ctx, client);
        let captured = Arc::new(Mutex::new(None));
        let handlers: Vec<Arc<dyn LimboHandler>> = vec![Arc::new(TokenCaptureHandler {
            name: "cap",
            result: HandlerResult::Accept,
            captured: Arc::clone(&captured),
        })];

        let exit = enter_limbo(
            &ctx,
            &mut io,
            handlers,
            LimboEntryContext::InitialConnection {
                target_server: ServerId::new("lobby"),
            },
            &ServerId::new("lobby"),
        )
        .await;

        assert!(matches!(exit, LimboExit::Completed));
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
