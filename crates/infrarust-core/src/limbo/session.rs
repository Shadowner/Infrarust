//! [`LimboSessionImpl`] -- sealed implementation of `LimboSession`.
//!
//! Bridges the API-level [`LimboSession`] trait to concrete packet encoding
//! and an mpsc channel that the limbo engine loop drains.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use infrarust_api::error::PlayerError;
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handle::SessionHandle;
use infrarust_api::limbo::handler::HandlerResult;
use infrarust_api::limbo::session::{LimboSession, private};
use infrarust_api::types::{Component, GameProfile, PlayerId, TitleData};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use crate::player::packets;

/// Concrete implementation of [`LimboSession`] used by the limbo engine.
///
/// Holds the player identity, a sender half of the outgoing packet channel,
/// and a watch channel used to signal handler completion.
pub(crate) struct LimboSessionImpl {
    player_id: PlayerId,
    profile: GameProfile,
    protocol_version: ProtocolVersion,
    entry_context: LimboEntryContext,
    client_sender: mpsc::Sender<PacketFrame>,
    complete_sender: watch::Sender<Option<HandlerResult>>,
    limbo_token: CancellationToken,
    hold_seq: AtomicU64,
    packet_registry: Arc<PacketRegistry>,
    self_ref: OnceLock<Weak<Self>>,
}

impl LimboSessionImpl {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        player_id: PlayerId,
        profile: GameProfile,
        protocol_version: ProtocolVersion,
        entry_context: LimboEntryContext,
        client_sender: mpsc::Sender<PacketFrame>,
        complete_sender: watch::Sender<Option<HandlerResult>>,
        limbo_token: CancellationToken,
        packet_registry: Arc<PacketRegistry>,
    ) -> Self {
        Self {
            player_id,
            profile,
            protocol_version,
            entry_context,
            client_sender,
            complete_sender,
            limbo_token,
            hold_seq: AtomicU64::new(0),
            packet_registry,
            self_ref: OnceLock::new(),
        }
    }

    pub(crate) fn set_self_ref(&self, weak: Weak<Self>) {
        let _ = self.self_ref.set(weak);
    }

    pub(crate) fn begin_handler(&self) {
        self.hold_seq.fetch_add(1, Ordering::SeqCst);
    }

    fn current_hold_id(&self) -> u64 {
        self.hold_seq.load(Ordering::SeqCst)
    }
}

impl private::Sealed for LimboSessionImpl {}

impl LimboSession for LimboSessionImpl {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn profile(&self) -> &GameProfile {
        &self.profile
    }

    fn entry_context(&self) -> &LimboEntryContext {
        &self.entry_context
    }

    fn send_message(&self, message: Component) -> Result<(), PlayerError> {
        let frame = packets::build_system_chat_message(
            &message,
            self.protocol_version,
            &self.packet_registry,
        )
        .map_err(|e| PlayerError::SendFailed(e.to_string()))?;

        self.client_sender
            .try_send(frame)
            .map_err(|e| PlayerError::SendFailed(e.to_string()))
    }

    fn send_title(&self, title: TitleData) -> Result<(), PlayerError> {
        let frames =
            packets::build_title_packets(&title, self.protocol_version, &self.packet_registry)
                .map_err(|e| PlayerError::SendFailed(e.to_string()))?;

        for frame in frames {
            self.client_sender
                .try_send(frame)
                .map_err(|e| PlayerError::SendFailed(e.to_string()))?;
        }
        Ok(())
    }

    fn send_action_bar(&self, message: Component) -> Result<(), PlayerError> {
        let frame =
            packets::build_action_bar(&message, self.protocol_version, &self.packet_registry)
                .map_err(|e| PlayerError::SendFailed(e.to_string()))?;

        self.client_sender
            .try_send(frame)
            .map_err(|e| PlayerError::SendFailed(e.to_string()))
    }

    fn complete(&self, result: HandlerResult) {
        self.complete_scoped(self.current_hold_id(), result);
    }

    fn complete_scoped(&self, hold_id: u64, result: HandlerResult) {
        if hold_id == self.current_hold_id() {
            let _ = self.complete_sender.send(Some(result));
        }
    }

    fn handle(&self) -> SessionHandle {
        let weak = self
            .self_ref
            .get()
            .expect("LimboSessionImpl::set_self_ref must be called before handle()");
        let arc = weak
            .upgrade()
            .expect("session Arc must be alive while session is in use");
        SessionHandle::new(arc as Arc<dyn LimboSession>, self.current_hold_id())
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.limbo_token.clone()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::super::test_helpers::test_profile;
    use super::*;
    use infrarust_api::limbo::handler::HandlerResult;
    use infrarust_api::types::PlayerId;
    use infrarust_protocol::version::ProtocolVersion;

    fn make_session() -> (
        LimboSessionImpl,
        mpsc::Receiver<PacketFrame>,
        watch::Receiver<Option<HandlerResult>>,
    ) {
        let (tx, rx) = mpsc::channel(64);
        let (complete_tx, complete_rx) = watch::channel(None);
        let registry = Arc::new(infrarust_protocol::registry::build_default_registry());

        let session = LimboSessionImpl::new(
            PlayerId::new(1),
            test_profile(),
            ProtocolVersion::V1_21,
            LimboEntryContext::PluginRedirect { from_server: None },
            tx,
            complete_tx,
            CancellationToken::new(),
            registry,
        );
        (session, rx, complete_rx)
    }

    #[test]
    fn player_id_returns_correct_value() {
        let (session, _rx, _crx) = make_session();
        assert_eq!(session.player_id(), PlayerId::new(1));
    }

    #[test]
    fn profile_returns_correct_username() {
        let (session, _rx, _crx) = make_session();
        assert_eq!(session.profile().username, "LimboTester");
    }

    #[test]
    fn entry_context_returns_plugin_redirect() {
        let (session, _rx, _crx) = make_session();
        assert!(matches!(
            session.entry_context(),
            LimboEntryContext::PluginRedirect { from_server: None }
        ));
    }

    #[test]
    fn send_message_pushes_to_channel() {
        let (session, mut rx, _crx) = make_session();
        let component = Component::text("Hello from limbo");
        session.send_message(component).unwrap();

        let frame = rx.try_recv().expect("should have received a frame");
        assert!(frame.id >= 0, "frame ID should be a valid packet ID");
    }

    #[test]
    fn send_action_bar_pushes_to_channel() {
        let (session, mut rx, _crx) = make_session();
        let component = Component::text("Action bar test");
        session.send_action_bar(component).unwrap();

        let frame = rx.try_recv().expect("should have received a frame");
        assert!(frame.id >= 0, "frame ID should be a valid packet ID");
    }

    #[test]
    fn complete_triggers_watch() {
        let (session, _rx, mut crx) = make_session();
        assert!(crx.borrow().is_none());

        session.complete(HandlerResult::Accept);

        // The watch should now have a value
        assert!(crx.borrow_and_update().is_some());
        match crx.borrow().as_ref().unwrap() {
            HandlerResult::Accept => {} // expected
            other => panic!("expected Accept, got {other:?}"),
        }
    }

    #[test]
    fn stale_handle_cannot_complete_a_later_hold() {
        let (session, _rx, mut crx) = make_session();
        let session = Arc::new(session);
        session.set_self_ref(Arc::downgrade(&session));

        session.begin_handler(); // generation 1
        let stale = session.handle(); // captures generation 1
        session.begin_handler(); // advance to generation 2 (a later handler)

        stale.complete(HandlerResult::Accept);
        assert!(
            crx.borrow_and_update().is_none(),
            "stale handle should be a no-op"
        );

        let current = session.handle(); // captures generation 2
        current.complete(HandlerResult::Accept);
        assert!(
            crx.borrow_and_update().is_some(),
            "current-generation handle should complete the Hold"
        );
    }

    #[test]
    fn send_message_fails_when_channel_closed() {
        let (tx, rx) = mpsc::channel(1);
        let (complete_tx, _complete_rx) = watch::channel(None);
        let registry = Arc::new(infrarust_protocol::registry::build_default_registry());

        let session = LimboSessionImpl::new(
            PlayerId::new(2),
            test_profile(),
            ProtocolVersion::V1_21,
            LimboEntryContext::PluginRedirect { from_server: None },
            tx,
            complete_tx,
            CancellationToken::new(),
            registry,
        );

        // Drop the receiver to close the channel
        drop(rx);

        let result = session.send_message(Component::text("should fail"));
        assert!(result.is_err());
    }
}
