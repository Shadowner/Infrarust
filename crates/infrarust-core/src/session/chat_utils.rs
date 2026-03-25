//! Chat/command detection helpers extracted from the proxy loop.
//!
//! Determines whether a serverbound packet is a chat message or slash command.

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::play::chat::{SChatCommand, SChatMessage};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

/// What a chat/command packet resolved to.
pub(crate) enum ChatAction {
    /// A slash command (without the leading `/`).
    Command(String),
    /// A regular chat message.
    Message(String),
}

/// Detects if a frame is a chat message or slash command.
///
/// Returns `Some(ChatAction)` if the frame matches a serverbound chat
/// packet (`SChatMessage` or `SChatCommand`), `None` otherwise.
pub(crate) fn detect_chat_or_command(
    frame: &PacketFrame,
    registry: &PacketRegistry,
    version: ProtocolVersion,
) -> Option<ChatAction> {
    // Check if it's a SChatCommand packet (1.19+)
    let chat_cmd_id = registry.get_packet_id::<SChatCommand>(
        ConnectionState::Play,
        Direction::Serverbound,
        version,
    );
    if Some(frame.id) == chat_cmd_id {
        // Decode just the command string
        let mut data = frame.payload.as_ref();
        if let Ok(decoded) = SChatCommand::decode(&mut data, version) {
            return Some(ChatAction::Command(decoded.command));
        }
    }

    // Check if it's a SChatMessage packet
    let chat_msg_id = registry.get_packet_id::<SChatMessage>(
        ConnectionState::Play,
        Direction::Serverbound,
        version,
    );
    if Some(frame.id) == chat_msg_id {
        // Decode just the message string
        let mut data = frame.payload.as_ref();
        if let Ok(decoded) = SChatMessage::decode(&mut data, version) {
            if decoded.message.starts_with('/') {
                // Pre-1.19 style: commands sent as chat messages with leading /
                return Some(ChatAction::Command(decoded.message[1..].to_string()));
            }
            return Some(ChatAction::Message(decoded.message));
        }
    }

    None
}
