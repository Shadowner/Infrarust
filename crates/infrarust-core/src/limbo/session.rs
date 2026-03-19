//! [`LimboSessionImpl`] -- sealed implementation of `LimboSession`.
//!
//! Bridges the API-level [`LimboSession`] trait to concrete packet encoding
//! and an mpsc channel that the limbo engine loop drains.

use std::sync::Arc;

use tokio::sync::{mpsc, watch};

use infrarust_api::error::PlayerError;
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
    client_sender: mpsc::Sender<PacketFrame>,
    complete_sender: watch::Sender<Option<HandlerResult>>,
    packet_registry: Arc<PacketRegistry>,
}

impl LimboSessionImpl {
    pub fn new(
        player_id: PlayerId,
        profile: GameProfile,
        protocol_version: ProtocolVersion,
        client_sender: mpsc::Sender<PacketFrame>,
        complete_sender: watch::Sender<Option<HandlerResult>>,
        packet_registry: Arc<PacketRegistry>,
    ) -> Self {
        Self {
            player_id,
            profile,
            protocol_version,
            client_sender,
            complete_sender,
            packet_registry,
        }
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
        let frames = packets::build_title_packets(
            &title,
            self.protocol_version,
            &self.packet_registry,
        )
        .map_err(|e| PlayerError::SendFailed(e.to_string()))?;

        for frame in frames {
            self.client_sender
                .try_send(frame)
                .map_err(|e| PlayerError::SendFailed(e.to_string()))?;
        }
        Ok(())
    }

    fn send_action_bar(&self, message: Component) -> Result<(), PlayerError> {
        let frame = packets::build_action_bar(
            &message,
            self.protocol_version,
            &self.packet_registry,
        )
        .map_err(|e| PlayerError::SendFailed(e.to_string()))?;

        self.client_sender
            .try_send(frame)
            .map_err(|e| PlayerError::SendFailed(e.to_string()))
    }

    fn complete(&self, result: HandlerResult) {
        let _ = self.complete_sender.send(Some(result));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use super::super::test_helpers::test_profile;
    use infrarust_api::limbo::handler::HandlerResult;
    use infrarust_api::types::PlayerId;
    use infrarust_protocol::version::ProtocolVersion;

    fn make_session() -> (LimboSessionImpl, mpsc::Receiver<PacketFrame>, watch::Receiver<Option<HandlerResult>>) {
        let (tx, rx) = mpsc::channel(64);
        let (complete_tx, complete_rx) = watch::channel(None);
        let registry = Arc::new(infrarust_protocol::registry::build_default_registry());

        let session = LimboSessionImpl::new(
            PlayerId::new(1),
            test_profile(),
            ProtocolVersion::V1_21,
            tx,
            complete_tx,
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
    fn send_message_fails_when_channel_closed() {
        let (tx, rx) = mpsc::channel(1);
        let (complete_tx, _complete_rx) = watch::channel(None);
        let registry = Arc::new(infrarust_protocol::registry::build_default_registry());

        let session = LimboSessionImpl::new(
            PlayerId::new(2),
            test_profile(),
            ProtocolVersion::V1_21,
            tx,
            complete_tx,
            registry,
        );

        // Drop the receiver to close the channel
        drop(rx);

        let result = session.send_message(Component::text("should fail"));
        assert!(result.is_err());
    }
}
