//! [`VirtualSessionCore`] -- shared plumbing for virtual connections.
//!
//! Provides the identity and outgoing packet channel that both
//! [`LimboSessionImpl`](super::session::LimboSessionImpl) and the limbo
//! engine loop use to communicate with the client.

use std::sync::Arc;

use tokio::sync::mpsc;

use infrarust_api::types::{GameProfile, PlayerId};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

/// Channel buffer size for outgoing packets queued by the limbo session.
const OUTGOING_CHANNEL_SIZE: usize = 64;

/// Shared plumbing for a player in limbo.
///
/// Owns the identity data, the protocol version, the packet registry,
/// and a bounded mpsc channel for outgoing frames. The engine loop
/// drains `outgoing_rx` and writes the frames to the client bridge.
pub(crate) struct VirtualSessionCore {
    pub player_id: PlayerId,
    pub profile: GameProfile,
    pub protocol_version: ProtocolVersion,
    pub packet_registry: Arc<PacketRegistry>,
    pub outgoing_tx: mpsc::Sender<PacketFrame>,
    pub outgoing_rx: mpsc::Receiver<PacketFrame>,
}

impl VirtualSessionCore {
    /// Creates a new virtual session core with a fresh outgoing channel.
    pub fn new(
        player_id: PlayerId,
        profile: GameProfile,
        protocol_version: ProtocolVersion,
        packet_registry: Arc<PacketRegistry>,
    ) -> Self {
        let (outgoing_tx, outgoing_rx) = mpsc::channel(OUTGOING_CHANNEL_SIZE);
        Self {
            player_id,
            profile,
            protocol_version,
            packet_registry,
            outgoing_tx,
            outgoing_rx,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::super::test_helpers::test_profile;
    use super::*;
    use infrarust_api::types::PlayerId;

    #[tokio::test]
    async fn outgoing_channel_works() {
        let registry = Arc::new(infrarust_protocol::registry::build_default_registry());
        let mut core = VirtualSessionCore::new(
            PlayerId::new(1),
            test_profile(),
            ProtocolVersion::V1_21,
            registry,
        );

        let frame = PacketFrame {
            id: 42,
            payload: bytes::Bytes::from_static(b"test"),
        };
        core.outgoing_tx.send(frame.clone()).await.unwrap();

        let received = core.outgoing_rx.recv().await.unwrap();
        assert_eq!(received.id, 42);
    }

    #[test]
    fn fields_accessible() {
        let registry = Arc::new(infrarust_protocol::registry::build_default_registry());
        let core = VirtualSessionCore::new(
            PlayerId::new(7),
            test_profile(),
            ProtocolVersion::V1_21,
            registry,
        );

        assert_eq!(core.player_id, PlayerId::new(7));
        assert_eq!(core.profile.username, "LimboTester");
        assert_eq!(core.protocol_version, ProtocolVersion::V1_21);
    }
}
