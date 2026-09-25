use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::play::chat::{
    LastSeenMessages, PreviousMessages, SChatAcknowledgement, SChatCommand, SChatCommandSigned,
    SChatMessage,
};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ChatIds {
    pub(crate) message: Option<i32>,
    pub(crate) command: Option<i32>,
    pub(crate) signed_command: Option<i32>,
    pub(crate) acknowledgement: Option<i32>,
}

impl ChatIds {
    pub(crate) fn resolve(registry: &PacketRegistry, version: ProtocolVersion) -> Self {
        Self {
            message: registry.get_packet_id::<SChatMessage>(version),
            command: registry.get_packet_id::<SChatCommand>(version),
            signed_command: registry.get_packet_id::<SChatCommandSigned>(version),
            acknowledgement: registry.get_packet_id::<SChatAcknowledgement>(version),
        }
    }
}

#[derive(Debug)]
pub(crate) enum PlayerInput {
    Chat(ChatInput),
    Command(CommandInput),
}

#[derive(Debug)]
pub(crate) struct ChatInput {
    packet: SChatMessage,
}

#[derive(Debug)]
pub(crate) struct CommandInput {
    command: String,
    packet: CommandPacket,
}

#[derive(Debug)]
enum CommandPacket {
    Legacy,
    Unsigned(SChatCommand),
    Signed(SChatCommandSigned),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outgoing {
    Message(SChatMessage),
    Command(SChatCommand),
}

pub(crate) fn decode_player_input(
    frame: &PacketFrame,
    ids: &ChatIds,
    version: ProtocolVersion,
) -> Option<PlayerInput> {
    let id = Some(frame.id);
    let mut data = frame.payload.as_ref();
    if id == ids.message {
        let packet = SChatMessage::decode(&mut data, version).ok()?;
        if version.less_than(ProtocolVersion::V1_19)
            && let Some(command) = packet.message.strip_prefix('/')
        {
            return Some(PlayerInput::Command(CommandInput {
                command: command.to_string(),
                packet: CommandPacket::Legacy,
            }));
        }
        return Some(PlayerInput::Chat(ChatInput { packet }));
    }
    if id == ids.command {
        let packet = SChatCommand::decode(&mut data, version).ok()?;
        return Some(PlayerInput::Command(CommandInput {
            command: packet.command.clone(),
            packet: CommandPacket::Unsigned(packet),
        }));
    }
    if id == ids.signed_command {
        let packet = SChatCommandSigned::decode(&mut data, version).ok()?;
        return Some(PlayerInput::Command(CommandInput {
            command: packet.command.clone(),
            packet: CommandPacket::Signed(packet),
        }));
    }
    None
}

fn acknowledgement(
    previous: &PreviousMessages,
    last_seen: &LastSeenMessages,
    version: ProtocolVersion,
) -> Option<SChatAcknowledgement> {
    if version.less_than(ProtocolVersion::V1_19_1) {
        return None;
    }
    if version.less_than(ProtocolVersion::V1_19_3) {
        let seen = !previous.seen.is_empty() || previous.last_received.is_some();
        return seen.then(|| SChatAcknowledgement {
            offset: 0,
            previous_messages: previous.clone(),
        });
    }
    (last_seen.offset > 0).then(|| SChatAcknowledgement {
        offset: last_seen.offset,
        previous_messages: PreviousMessages::default(),
    })
}

impl ChatInput {
    pub(crate) fn message(&self) -> &str {
        &self.packet.message
    }

    pub(crate) fn into_message(self) -> String {
        self.packet.message
    }

    pub(crate) fn signed(&self) -> bool {
        self.packet.signature.is_some()
    }

    pub(crate) fn acknowledgement(&self, version: ProtocolVersion) -> Option<SChatAcknowledgement> {
        acknowledgement(
            &self.packet.previous_messages,
            &self.packet.last_seen,
            version,
        )
    }

    pub(crate) fn rewrite(&self, message: String) -> SChatMessage {
        SChatMessage {
            message,
            signature: None,
            signed_preview: false,
            ..self.packet.clone()
        }
    }
}

impl CommandInput {
    pub(crate) fn command(&self) -> &str {
        &self.command
    }

    pub(crate) fn into_command(self) -> String {
        self.command
    }

    pub(crate) fn signed(&self) -> bool {
        match &self.packet {
            CommandPacket::Legacy => false,
            CommandPacket::Unsigned(packet) => packet
                .argument_signatures
                .iter()
                .any(|argument| !argument.signature.is_empty()),
            CommandPacket::Signed(packet) => packet
                .argument_signatures
                .iter()
                .any(|argument| !argument.signature.is_empty()),
        }
    }

    pub(crate) fn acknowledgement(&self, version: ProtocolVersion) -> Option<SChatAcknowledgement> {
        match &self.packet {
            CommandPacket::Legacy => None,
            CommandPacket::Unsigned(packet) => {
                acknowledgement(&packet.previous_messages, &packet.last_seen, version)
            }
            CommandPacket::Signed(packet) => {
                acknowledgement(&PreviousMessages::default(), &packet.last_seen, version)
            }
        }
    }

    pub(crate) fn rewrite(
        &self,
        command: &str,
        version: ProtocolVersion,
    ) -> (Outgoing, Option<SChatAcknowledgement>) {
        match &self.packet {
            CommandPacket::Legacy => (
                Outgoing::Message(SChatMessage {
                    message: format!("/{command}"),
                    ..SChatMessage::default()
                }),
                None,
            ),
            CommandPacket::Unsigned(packet) => (
                Outgoing::Command(SChatCommand {
                    command: command.to_string(),
                    argument_signatures: Vec::new(),
                    signed_preview: false,
                    ..packet.clone()
                }),
                None,
            ),
            CommandPacket::Signed(_) => (
                Outgoing::Command(SChatCommand {
                    command: command.to_string(),
                    ..SChatCommand::default()
                }),
                self.acknowledgement(version),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use bytes::Bytes;
    use infrarust_protocol::packets::play::chat::{ArgumentSignature, PreviousMessage};
    use infrarust_protocol::registry::build_default_registry;

    use super::*;

    const SIGNATURE: [u8; 256] = [7; 256];

    fn frame<P: Packet>(packet: &P, id: Option<i32>, version: ProtocolVersion) -> PacketFrame {
        let mut payload = Vec::new();
        packet.encode(&mut payload, version).unwrap();
        PacketFrame::new(id.unwrap(), Bytes::from(payload))
    }

    fn decode(
        packet: &impl Packet,
        pick: fn(&ChatIds) -> Option<i32>,
        version: ProtocolVersion,
    ) -> PlayerInput {
        let ids = ChatIds::resolve(&build_default_registry(), version);
        decode_player_input(&frame(packet, pick(&ids), version), &ids, version).unwrap()
    }

    fn last_seen(offset: i32) -> LastSeenMessages {
        LastSeenMessages {
            offset,
            acknowledged: [1, 2, 3],
            checksum: 9,
        }
    }

    fn signatures() -> Vec<ArgumentSignature> {
        vec![ArgumentSignature {
            name: "message".into(),
            signature: SIGNATURE.to_vec(),
        }]
    }

    fn command(input: PlayerInput) -> CommandInput {
        match input {
            PlayerInput::Command(command) => command,
            PlayerInput::Chat(chat) => panic!("expected a command, got {chat:?}"),
        }
    }

    fn chat(input: PlayerInput) -> ChatInput {
        match input {
            PlayerInput::Chat(chat) => chat,
            PlayerInput::Command(command) => panic!("expected a chat message, got {command:?}"),
        }
    }

    #[test]
    fn a_slash_is_a_command_only_before_1_19() {
        let slash = SChatMessage {
            message: "/spawn now".into(),
            ..SChatMessage::default()
        };
        let legacy = command(decode(&slash, |ids| ids.message, ProtocolVersion::V1_18_2));
        assert_eq!(legacy.command(), "spawn now");
        assert!(!legacy.signed());
        assert_eq!(legacy.acknowledgement(ProtocolVersion::V1_18_2), None);

        let text = chat(decode(&slash, |ids| ids.message, ProtocolVersion::V1_19_4));
        assert_eq!(text.message(), "/spawn now");
    }

    #[test]
    fn every_command_packet_is_recognised() {
        let version = ProtocolVersion::V1_21_5;
        let unsigned = SChatCommand {
            command: "spawn".into(),
            ..SChatCommand::default()
        };
        let signed = SChatCommandSigned {
            command: "msg Alex hi".into(),
            argument_signatures: signatures(),
            last_seen: last_seen(4),
            ..SChatCommandSigned::default()
        };

        let plain = command(decode(&unsigned, |ids| ids.command, version));
        assert_eq!((plain.command(), plain.signed()), ("spawn", false));
        assert_eq!(plain.acknowledgement(version), None);

        let signed = command(decode(&signed, |ids| ids.signed_command, version));
        assert_eq!((signed.command(), signed.signed()), ("msg Alex hi", true));
        assert_eq!(
            signed.acknowledgement(version),
            Some(SChatAcknowledgement {
                offset: 4,
                ..SChatAcknowledgement::default()
            })
        );
    }

    #[test]
    fn a_signed_command_without_signatures_is_not_signed() {
        let version = ProtocolVersion::V1_21_11;
        let packet = SChatCommandSigned {
            command: "msg Alex hi".into(),
            ..SChatCommandSigned::default()
        };
        assert!(!command(decode(&packet, |ids| ids.signed_command, version)).signed());
    }

    #[test]
    fn acknowledgements_follow_each_format() {
        let previous = PreviousMessages {
            seen: vec![PreviousMessage {
                sender: uuid::Uuid::nil(),
                signature: vec![1, 2],
            }],
            last_received: None,
        };
        let message = SChatMessage {
            message: "hi".into(),
            previous_messages: previous.clone(),
            last_seen: last_seen(2),
            ..SChatMessage::default()
        };

        let v1_19 = chat(decode(&message, |ids| ids.message, ProtocolVersion::V1_19));
        assert_eq!(v1_19.acknowledgement(ProtocolVersion::V1_19), None);

        let v1_19_2 = chat(decode(
            &message,
            |ids| ids.message,
            ProtocolVersion::V1_19_1,
        ));
        assert_eq!(
            v1_19_2.acknowledgement(ProtocolVersion::V1_19_1),
            Some(SChatAcknowledgement {
                offset: 0,
                previous_messages: previous,
            })
        );

        let modern = chat(decode(
            &message,
            |ids| ids.message,
            ProtocolVersion::V1_20_3,
        ));
        assert_eq!(
            modern.acknowledgement(ProtocolVersion::V1_20_3),
            Some(SChatAcknowledgement {
                offset: 2,
                ..SChatAcknowledgement::default()
            })
        );

        let nothing_seen = SChatMessage {
            last_seen: last_seen(0),
            ..message
        };
        let quiet = chat(decode(
            &nothing_seen,
            |ids| ids.message,
            ProtocolVersion::V1_20_3,
        ));
        assert_eq!(quiet.acknowledgement(ProtocolVersion::V1_20_3), None);
    }

    #[test]
    fn a_rewritten_message_keeps_its_trailer_without_the_signature() {
        let version = ProtocolVersion::V1_21_5;
        let original = SChatMessage {
            message: "hello".into(),
            timestamp: 42,
            salt: 7,
            signature: Some(SIGNATURE.to_vec()),
            last_seen: last_seen(3),
            ..SChatMessage::default()
        };
        let input = chat(decode(&original, |ids| ids.message, version));
        assert!(input.signed());
        assert_eq!(
            input.rewrite("bye".into()),
            SChatMessage {
                message: "bye".into(),
                signature: None,
                ..original
            }
        );
    }

    #[test]
    fn a_rewritten_command_is_unsigned_in_every_format() {
        let legacy = SChatMessage {
            message: "/spawn".into(),
            ..SChatMessage::default()
        };
        let input = command(decode(&legacy, |ids| ids.message, ProtocolVersion::V1_12_2));
        assert_eq!(
            input.rewrite("hub", ProtocolVersion::V1_12_2),
            (
                Outgoing::Message(SChatMessage {
                    message: "/hub".into(),
                    ..SChatMessage::default()
                }),
                None
            )
        );

        let version = ProtocolVersion::V1_20_3;
        let keyed = SChatCommand {
            command: "msg Alex hi".into(),
            timestamp: 42,
            salt: 7,
            argument_signatures: signatures(),
            last_seen: LastSeenMessages {
                checksum: 0,
                ..last_seen(3)
            },
            ..SChatCommand::default()
        };
        let input = command(decode(&keyed, |ids| ids.command, version));
        assert!(input.signed());
        assert_eq!(
            input.rewrite("msg Alex bye", version),
            (
                Outgoing::Command(SChatCommand {
                    command: "msg Alex bye".into(),
                    argument_signatures: Vec::new(),
                    ..keyed
                }),
                None
            )
        );

        let version = ProtocolVersion::V1_21_5;
        let signed = SChatCommandSigned {
            command: "msg Alex hi".into(),
            timestamp: 42,
            salt: 7,
            argument_signatures: signatures(),
            last_seen: last_seen(3),
        };
        let input = command(decode(&signed, |ids| ids.signed_command, version));
        assert_eq!(
            input.rewrite("msg Alex bye", version),
            (
                Outgoing::Command(SChatCommand {
                    command: "msg Alex bye".into(),
                    ..SChatCommand::default()
                }),
                Some(SChatAcknowledgement {
                    offset: 3,
                    ..SChatAcknowledgement::default()
                })
            )
        );
    }
}
