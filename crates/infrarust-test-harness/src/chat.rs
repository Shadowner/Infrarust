use std::time::{SystemTime, UNIX_EPOCH};

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::play::chat::{
    ArgumentSignature, LastSeenMessages, PreviousMessage, PreviousMessages, SChatAcknowledgement,
    SChatCommand, SChatCommandSigned, SChatMessage,
};
use infrarust_protocol::version::ProtocolVersion;
use uuid::Uuid;

use crate::error::HarnessResult;
use crate::wire;

pub const SIGNATURE: [u8; 256] = [0x5A; 256];

pub const SALT: i64 = 0x0123_4567_89AB_CDEF;

pub const ACKNOWLEDGED: [u8; 3] = [0x05, 0x00, 0x80];

pub const CHECKSUM: u8 = 0x2A;

const SEEN_SENDER: Uuid = Uuid::from_u128(0x1111_2222_3333_4444_5555_6666_7777_8888);

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

pub fn has_acknowledgements(version: ProtocolVersion) -> bool {
    version.no_less_than(ProtocolVersion::V1_19_1)
}

pub fn last_seen(offset: i32, version: ProtocolVersion) -> LastSeenMessages {
    if version.less_than(ProtocolVersion::V1_19_3) {
        return LastSeenMessages::default();
    }
    LastSeenMessages {
        offset,
        acknowledged: ACKNOWLEDGED,
        checksum: if version.no_less_than(ProtocolVersion::V1_21_5) {
            CHECKSUM
        } else {
            0
        },
    }
}

pub fn previous_messages(offset: i32, version: ProtocolVersion) -> PreviousMessages {
    if offset <= 0
        || version.less_than(ProtocolVersion::V1_19_1)
        || version.no_less_than(ProtocolVersion::V1_19_3)
    {
        return PreviousMessages::default();
    }
    PreviousMessages {
        seen: vec![PreviousMessage {
            sender: SEEN_SENDER,
            signature: SIGNATURE.to_vec(),
        }],
        last_received: None,
    }
}

pub fn acknowledgement(offset: i32, version: ProtocolVersion) -> Option<SChatAcknowledgement> {
    if offset <= 0 || !has_acknowledgements(version) {
        return None;
    }
    Some(SChatAcknowledgement {
        offset: if version.no_less_than(ProtocolVersion::V1_19_3) {
            offset
        } else {
            0
        },
        previous_messages: previous_messages(offset, version),
    })
}

pub fn unsigned_chat(message: &str, offset: i32, version: ProtocolVersion) -> SChatMessage {
    let mut packet = SChatMessage {
        message: message.to_string(),
        ..SChatMessage::default()
    };
    if version.no_less_than(ProtocolVersion::V1_19) {
        packet.timestamp = now_millis();
        packet.salt = SALT;
        packet.previous_messages = previous_messages(offset, version);
        packet.last_seen = last_seen(offset, version);
    }
    packet
}

pub fn signed_chat(message: &str, offset: i32, version: ProtocolVersion) -> SChatMessage {
    let mut packet = unsigned_chat(message, offset, version);
    if version.no_less_than(ProtocolVersion::V1_19) {
        packet.signature = Some(SIGNATURE.to_vec());
    }
    packet
}

pub fn argument_signatures(version: ProtocolVersion) -> Vec<ArgumentSignature> {
    if version.less_than(ProtocolVersion::V1_19) {
        return Vec::new();
    }
    vec![ArgumentSignature {
        name: "message".to_string(),
        signature: SIGNATURE.to_vec(),
    }]
}

pub fn signed_command_frame(
    command: &str,
    offset: i32,
    version: ProtocolVersion,
) -> HarnessResult<PacketFrame> {
    let command = command.strip_prefix('/').unwrap_or(command);
    if version.less_than(ProtocolVersion::V1_19) {
        return wire::encode(&unsigned_chat(&format!("/{command}"), 0, version), version);
    }
    if version.no_less_than(ProtocolVersion::V1_20_5) {
        let packet = SChatCommandSigned {
            command: command.to_string(),
            timestamp: now_millis(),
            salt: SALT,
            argument_signatures: argument_signatures(version),
            last_seen: last_seen(offset, version),
        };
        return wire::encode(&packet, version);
    }
    let packet = SChatCommand {
        command: command.to_string(),
        timestamp: now_millis(),
        salt: SALT,
        argument_signatures: argument_signatures(version),
        signed_preview: false,
        previous_messages: previous_messages(offset, version),
        last_seen: last_seen(offset, version),
    };
    wire::encode(&packet, version)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatPacket {
    Message(SChatMessage),
    Command(SChatCommand),
    SignedCommand(SChatCommandSigned),
    Acknowledgement(SChatAcknowledgement),
}

#[derive(Debug, Clone)]
pub struct ChatFrame {
    pub frame: PacketFrame,
    pub packet: ChatPacket,
}

impl ChatFrame {
    pub fn classify(frame: &PacketFrame, version: ProtocolVersion) -> HarnessResult<Option<Self>> {
        let packet = if wire::is::<SChatMessage>(frame, version) {
            ChatPacket::Message(wire::decode(frame, version)?)
        } else if wire::is::<SChatCommand>(frame, version) {
            ChatPacket::Command(wire::decode(frame, version)?)
        } else if wire::is::<SChatCommandSigned>(frame, version) {
            ChatPacket::SignedCommand(wire::decode(frame, version)?)
        } else if wire::is::<SChatAcknowledgement>(frame, version) {
            ChatPacket::Acknowledgement(wire::decode(frame, version)?)
        } else {
            return Ok(None);
        };
        Ok(Some(Self {
            frame: frame.clone(),
            packet,
        }))
    }

    pub fn is_message(&self, text: &str) -> bool {
        matches!(&self.packet, ChatPacket::Message(message) if message.message == text)
    }

    pub fn command(&self) -> Option<&str> {
        match &self.packet {
            ChatPacket::Message(message) => message.message.strip_prefix('/'),
            ChatPacket::Command(command) => Some(&command.command),
            ChatPacket::SignedCommand(command) => Some(&command.command),
            ChatPacket::Acknowledgement(_) => None,
        }
    }
}
