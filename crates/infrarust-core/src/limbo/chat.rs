//! Client message parsing for the Limbo loop.
//!
//! Wraps the shared [`decode_player_input`](crate::session::chat_utils::decode_player_input)
//! helper and splits commands into name + arguments for handler dispatch.

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use crate::session::chat_utils::{ChatIds, PlayerInput, decode_player_input};

/// A parsed client message — either a command with arguments or plain chat.
#[derive(Debug)]
pub(crate) enum ClientMessage {
    /// A slash command, split into name and arguments.
    Command { name: String, args: Vec<String> },
    /// A regular chat message.
    Chat { message: String, signed: bool },
}

/// Parses a serverbound frame into a [`ClientMessage`], if applicable.
///
/// Returns `None` if the frame is not a chat or command packet.
pub(crate) fn parse_client_message(
    frame: &PacketFrame,
    registry: &PacketRegistry,
    version: ProtocolVersion,
) -> Option<ClientMessage> {
    let ids = ChatIds::resolve(registry, version);
    match decode_player_input(frame, &ids, version)? {
        PlayerInput::Command(command) => {
            let input = command.into_command();
            let mut parts = input.splitn(2, ' ');
            let name = parts.next()?.to_string();
            let args = parts.next().map_or_else(Vec::new, |rest| {
                rest.split_whitespace().map(String::from).collect()
            });
            Some(ClientMessage::Command { name, args })
        }
        PlayerInput::Chat(chat) => {
            let signed = chat.signed();
            Some(ClientMessage::Chat {
                message: chat.into_message(),
                signed,
            })
        }
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
            ..SChatCommand::default()
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
            ..SChatCommand::default()
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
            ..SChatMessage::default()
        };
        let frame = build_frame(&pkt, version, &registry);

        let result = parse_client_message(&frame, &registry, version);
        match result {
            Some(ClientMessage::Chat { message, signed }) => {
                assert_eq!(message, "hello");
                assert!(!signed);
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
