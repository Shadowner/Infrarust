//! Client message parsing for the Limbo loop.
//!
//! Wraps the shared [`detect_chat_or_command`](crate::session::chat_utils::detect_chat_or_command)
//! helper and splits commands into name + arguments for handler dispatch.

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::play::chat::{SChatCommand, SChatMessage};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::session::chat_utils::{ChatAction, detect_chat_or_command};

/// A parsed client message — either a command with arguments or plain chat.
#[derive(Debug)]
pub(crate) enum ClientMessage {
    /// A slash command, split into name and arguments.
    Command { name: String, args: Vec<String> },
    /// A regular chat message.
    Chat { message: String },
}

/// Parses a serverbound frame into a [`ClientMessage`], if applicable.
///
/// Returns `None` if the frame is not a chat or command packet.
pub(crate) fn parse_client_message(
    frame: &PacketFrame,
    registry: &PacketRegistry,
    version: ProtocolVersion,
) -> Option<ClientMessage> {
    let chat_cmd_id = registry.get_packet_id::<SChatCommand>(
        ConnectionState::Play,
        Direction::Serverbound,
        version,
    );
    let chat_msg_id = registry.get_packet_id::<SChatMessage>(
        ConnectionState::Play,
        Direction::Serverbound,
        version,
    );
    let action = detect_chat_or_command(frame, chat_cmd_id, chat_msg_id, version)?;

    match action {
        ChatAction::Command(input) => {
            let mut parts = input.splitn(2, ' ');
            let name = parts.next()?.to_string();
            let args = parts.next().map_or_else(Vec::new, |rest| {
                rest.split_whitespace().map(String::from).collect()
            });
            Some(ClientMessage::Command { name, args })
        }
        ChatAction::Message(msg) => Some(ClientMessage::Chat { message: msg }),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::super::test_helpers::{build_frame, test_registry};
    use super::*;
    use bytes::Bytes;
    use infrarust_protocol::io::PacketFrame;
    use infrarust_protocol::packets::play::chat::{SChatCommand, SChatMessage};
    use infrarust_protocol::version::ProtocolVersion;

    #[test]
    fn command_with_args() {
        let registry = test_registry();
        let version = ProtocolVersion::V1_21;

        let pkt = SChatCommand {
            command: "login password".to_string(),
            remaining: vec![],
        };
        let frame = build_frame(&pkt, version, &registry);

        let result = parse_client_message(&frame, &registry, version);
        match result {
            Some(ClientMessage::Command { name, args }) => {
                assert_eq!(name, "login");
                assert_eq!(args, vec!["password"]);
            }
            other => panic!("expected Command, got {other:?}"),
        }
    }

    #[test]
    fn command_without_args() {
        let registry = test_registry();
        let version = ProtocolVersion::V1_21;

        let pkt = SChatCommand {
            command: "help".to_string(),
            remaining: vec![],
        };
        let frame = build_frame(&pkt, version, &registry);

        let result = parse_client_message(&frame, &registry, version);
        match result {
            Some(ClientMessage::Command { name, args }) => {
                assert_eq!(name, "help");
                assert!(args.is_empty());
            }
            other => panic!("expected Command, got {other:?}"),
        }
    }

    #[test]
    fn chat_message() {
        let registry = test_registry();
        let version = ProtocolVersion::V1_21;

        let pkt = SChatMessage {
            message: "hello".to_string(),
            remaining: vec![],
        };
        let frame = build_frame(&pkt, version, &registry);

        let result = parse_client_message(&frame, &registry, version);
        match result {
            Some(ClientMessage::Chat { message }) => {
                assert_eq!(message, "hello");
            }
            other => panic!("expected Chat, got {other:?}"),
        }
    }

    #[test]
    fn unrelated_packet_returns_none() {
        let registry = test_registry();
        let version = ProtocolVersion::V1_21;

        let frame = PacketFrame::new(9999, Bytes::new());

        assert!(parse_client_message(&frame, &registry, version).is_none());
    }
}
