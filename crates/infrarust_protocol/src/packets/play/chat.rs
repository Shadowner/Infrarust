use std::io::{Read, Write};

use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::{ProtocolError, ProtocolResult};
use crate::packets::play::common::{read_text_component, write_text_component};
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

const MESSAGE_SIGNATURE_LEN: usize = 256;

const ACKNOWLEDGED_LEN: usize = 3;

const ARGUMENT_NAME_MAX_CHARS: usize = 16;

#[derive(Debug, Clone)]
pub struct CSystemChatMessage {
    pub content: Vec<u8>,
    pub overlay: bool,
}

impl CSystemChatMessage {
    pub fn from_json(json: &str, overlay: bool) -> Self {
        Self {
            content: json.as_bytes().to_vec(),
            overlay,
        }
    }

    pub fn from_nbt(nbt: Vec<u8>, overlay: bool) -> Self {
        Self {
            content: nbt,
            overlay,
        }
    }
}

impl Packet for CSystemChatMessage {
    const NAME: &'static str = "CSystemChatMessage";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_19   => 0x5F,
        V1_19_1 => 0x62,
        V1_19_3 => 0x60,
        V1_19_4 => 0x64,
        V1_20_2 => 0x67,
        V1_20_3 => 0x69,
        V1_20_5 => 0x6C,
        V1_21_2 => 0x73,
        V1_21_5 => 0x72,
        V1_21_9 => 0x77,
        V26_1   => 0x79,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let content = read_text_component(r, version, 1, Self::NAME)?;
        let overlay = r.read_bool()?;
        Ok(Self { content, overlay })
    }

    fn encode(
        &self,
        mut w: &mut (impl Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        write_text_component(w, &self.content, version, Self::NAME, "content")?;
        w.write_bool(self.overlay)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CChatMessageLegacy {
    pub content: String,
    pub position: u8,
}

impl Packet for CChatMessageLegacy {
    const NAME: &'static str = "CChatMessageLegacy";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_7_2 => 0x02,
        V1_9   => 0x0F,
        V1_13  => 0x0E,
        V1_15  => 0x0F,
        V1_16  => 0x0E,
        V1_17 ..= V1_18_2 => 0x0F,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let content = r.read_string()?;
        let position = if version.no_less_than(ProtocolVersion::V1_8) {
            r.read_u8()?
        } else {
            0
        };
        if version.no_less_than(ProtocolVersion::V1_16) {
            let _ = r.read_uuid()?;
        }
        Ok(Self { content, position })
    }

    fn encode(
        &self,
        mut w: &mut (impl Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_string(&self.content)?;
        if version.no_less_than(ProtocolVersion::V1_8) {
            w.write_u8(self.position)?;
        }
        if version.no_less_than(ProtocolVersion::V1_16) {
            w.write_uuid(&uuid::Uuid::nil())?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviousMessage {
    pub sender: uuid::Uuid,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviousMessages {
    pub seen: Vec<PreviousMessage>,
    pub last_received: Option<PreviousMessage>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LastSeenMessages {
    pub offset: i32,
    pub acknowledged: [u8; ACKNOWLEDGED_LEN],
    pub checksum: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgumentSignature {
    pub name: String,
    pub signature: Vec<u8>,
}

fn read_count(r: &mut &[u8], what: &str) -> ProtocolResult<usize> {
    let count = r.read_var_int()?.0;
    usize::try_from(count).map_err(|_| ProtocolError::invalid(format!("negative {what} count")))
}

fn read_prefixed_signature(r: &mut &[u8]) -> ProtocolResult<Vec<u8>> {
    let available = r.len();
    r.read_byte_array(available)
}

fn read_fixed_signature(r: &mut &[u8]) -> ProtocolResult<Vec<u8>> {
    r.read_byte_array_bounded(MESSAGE_SIGNATURE_LEN)
}

fn write_fixed_signature(
    w: &mut (impl Write + ?Sized),
    signature: &[u8],
    packet_name: &str,
) -> ProtocolResult<()> {
    if signature.len() != MESSAGE_SIGNATURE_LEN {
        return Err(ProtocolError::invalid(format!(
            "{packet_name}: a message signature is {MESSAGE_SIGNATURE_LEN} bytes, got {}",
            signature.len()
        )));
    }
    w.write_all(signature)?;
    Ok(())
}

fn decode_previous_message(r: &mut &[u8]) -> ProtocolResult<PreviousMessage> {
    let sender = r.read_uuid()?;
    let signature = read_prefixed_signature(r)?;
    Ok(PreviousMessage { sender, signature })
}

fn encode_previous_message(
    mut w: &mut (impl Write + ?Sized),
    message: &PreviousMessage,
) -> ProtocolResult<()> {
    w.write_uuid(&message.sender)?;
    w.write_byte_array(&message.signature)
}

fn decode_previous_messages(r: &mut &[u8]) -> ProtocolResult<PreviousMessages> {
    let count = read_count(r, "previous message")?;
    let mut seen = Vec::with_capacity(count.min(16));
    for _ in 0..count {
        seen.push(decode_previous_message(r)?);
    }
    let last_received = if r.read_bool()? {
        Some(decode_previous_message(r)?)
    } else {
        None
    };
    Ok(PreviousMessages {
        seen,
        last_received,
    })
}

fn encode_previous_messages(
    mut w: &mut (impl Write + ?Sized),
    messages: &PreviousMessages,
) -> ProtocolResult<()> {
    w.write_var_int(&VarInt(messages.seen.len() as i32))?;
    for message in &messages.seen {
        encode_previous_message(w, message)?;
    }
    match &messages.last_received {
        Some(message) => {
            w.write_bool(true)?;
            encode_previous_message(w, message)
        }
        None => w.write_bool(false),
    }
}

fn decode_last_seen(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<LastSeenMessages> {
    let offset = r.read_var_int()?.0;
    let mut acknowledged = [0u8; ACKNOWLEDGED_LEN];
    r.read_exact(&mut acknowledged)?;
    let checksum = if version.no_less_than(ProtocolVersion::V1_21_5) {
        r.read_u8()?
    } else {
        0
    };
    Ok(LastSeenMessages {
        offset,
        acknowledged,
        checksum,
    })
}

fn encode_last_seen(
    mut w: &mut (impl Write + ?Sized),
    last_seen: &LastSeenMessages,
    version: ProtocolVersion,
) -> ProtocolResult<()> {
    w.write_var_int(&VarInt(last_seen.offset))?;
    w.write_all(&last_seen.acknowledged)?;
    if version.no_less_than(ProtocolVersion::V1_21_5) {
        w.write_u8(last_seen.checksum)?;
    }
    Ok(())
}

fn decode_argument_signatures(
    r: &mut &[u8],
    version: ProtocolVersion,
) -> ProtocolResult<Vec<ArgumentSignature>> {
    let count = read_count(r, "argument signature")?;
    let mut signatures = Vec::with_capacity(count.min(8));
    for _ in 0..count {
        let name = r.read_string_bounded(ARGUMENT_NAME_MAX_CHARS)?;
        let signature = if version.no_less_than(ProtocolVersion::V1_19_3) {
            read_fixed_signature(r)?
        } else {
            read_prefixed_signature(r)?
        };
        signatures.push(ArgumentSignature { name, signature });
    }
    Ok(signatures)
}

fn encode_argument_signatures(
    mut w: &mut (impl Write + ?Sized),
    signatures: &[ArgumentSignature],
    version: ProtocolVersion,
    packet_name: &str,
) -> ProtocolResult<()> {
    w.write_var_int(&VarInt(signatures.len() as i32))?;
    for signature in signatures {
        w.write_string(&signature.name)?;
        if version.no_less_than(ProtocolVersion::V1_19_3) {
            write_fixed_signature(w, &signature.signature, packet_name)?;
        } else {
            w.write_byte_array(&signature.signature)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SChatMessage {
    pub message: String,
    pub timestamp: i64,
    pub salt: i64,
    pub signature: Option<Vec<u8>>,
    pub signed_preview: bool,
    pub previous_messages: PreviousMessages,
    pub last_seen: LastSeenMessages,
}

impl Packet for SChatMessage {
    const NAME: &'static str = "SChatMessage";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Serverbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_7_2  => 0x01,
        V1_9    => 0x02,
        V1_12   => 0x03,
        V1_12_1 => 0x02,
        V1_14   => 0x03,
        V1_19   => 0x04,
        V1_19_1 => 0x05,
        V1_20_5 => 0x06,
        V1_21_2 => 0x07,
        V1_21_6 => 0x08,
        V26_1   => 0x09,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let message = r.read_string()?;
        if version.less_than(ProtocolVersion::V1_19) {
            return Ok(Self {
                message,
                ..Self::default()
            });
        }

        let timestamp = r.read_i64_be()?;
        let salt = r.read_i64_be()?;

        if version.less_than(ProtocolVersion::V1_19_3) {
            let signature = Some(read_prefixed_signature(r)?).filter(|s| !s.is_empty());
            let signed_preview = r.read_bool()?;
            let previous_messages = if version.no_less_than(ProtocolVersion::V1_19_1) {
                decode_previous_messages(r)?
            } else {
                PreviousMessages::default()
            };
            return Ok(Self {
                message,
                timestamp,
                salt,
                signature,
                signed_preview,
                previous_messages,
                last_seen: LastSeenMessages::default(),
            });
        }

        let signature = if r.read_bool()? {
            Some(read_fixed_signature(r)?)
        } else {
            None
        };
        let last_seen = decode_last_seen(r, version)?;
        Ok(Self {
            message,
            timestamp,
            salt,
            signature,
            signed_preview: false,
            previous_messages: PreviousMessages::default(),
            last_seen,
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_string(&self.message)?;
        if version.less_than(ProtocolVersion::V1_19) {
            return Ok(());
        }

        w.write_i64_be(self.timestamp)?;
        w.write_i64_be(self.salt)?;

        if version.less_than(ProtocolVersion::V1_19_3) {
            w.write_byte_array(self.signature.as_deref().unwrap_or_default())?;
            w.write_bool(self.signed_preview)?;
            if version.no_less_than(ProtocolVersion::V1_19_1) {
                encode_previous_messages(w, &self.previous_messages)?;
            }
            return Ok(());
        }

        match &self.signature {
            Some(signature) => {
                w.write_bool(true)?;
                write_fixed_signature(w, signature, Self::NAME)?;
            }
            None => w.write_bool(false)?,
        }
        encode_last_seen(w, &self.last_seen, version)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SChatCommand {
    pub command: String,
    pub timestamp: i64,
    pub salt: i64,
    pub argument_signatures: Vec<ArgumentSignature>,
    pub signed_preview: bool,
    pub previous_messages: PreviousMessages,
    pub last_seen: LastSeenMessages,
}

impl Packet for SChatCommand {
    const NAME: &'static str = "SChatCommand";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Serverbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_19   => 0x03,
        V1_19_1 => 0x04,
        V1_21_2 => 0x05,
        V1_21_6 => 0x06,
        V26_1   => 0x07,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let command = r.read_string()?;
        if version.no_less_than(ProtocolVersion::V1_20_5) {
            return Ok(Self {
                command,
                ..Self::default()
            });
        }

        let timestamp = r.read_i64_be()?;
        let salt = r.read_i64_be()?;
        let argument_signatures = decode_argument_signatures(r, version)?;

        if version.less_than(ProtocolVersion::V1_19_3) {
            let signed_preview = r.read_bool()?;
            let previous_messages = if version.no_less_than(ProtocolVersion::V1_19_1) {
                decode_previous_messages(r)?
            } else {
                PreviousMessages::default()
            };
            return Ok(Self {
                command,
                timestamp,
                salt,
                argument_signatures,
                signed_preview,
                previous_messages,
                last_seen: LastSeenMessages::default(),
            });
        }

        let last_seen = decode_last_seen(r, version)?;
        Ok(Self {
            command,
            timestamp,
            salt,
            argument_signatures,
            signed_preview: false,
            previous_messages: PreviousMessages::default(),
            last_seen,
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_string(&self.command)?;
        if version.no_less_than(ProtocolVersion::V1_20_5) {
            return Ok(());
        }

        w.write_i64_be(self.timestamp)?;
        w.write_i64_be(self.salt)?;
        encode_argument_signatures(w, &self.argument_signatures, version, Self::NAME)?;

        if version.less_than(ProtocolVersion::V1_19_3) {
            w.write_bool(self.signed_preview)?;
            if version.no_less_than(ProtocolVersion::V1_19_1) {
                encode_previous_messages(w, &self.previous_messages)?;
            }
            return Ok(());
        }

        encode_last_seen(w, &self.last_seen, version)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SChatCommandSigned {
    pub command: String,
    pub timestamp: i64,
    pub salt: i64,
    pub argument_signatures: Vec<ArgumentSignature>,
    pub last_seen: LastSeenMessages,
}

impl Packet for SChatCommandSigned {
    const NAME: &'static str = "SChatCommandSigned";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Serverbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_20_5 => 0x05,
        V1_21_2 => 0x06,
        V1_21_6 => 0x07,
        V26_1   => 0x08,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let command = r.read_string()?;
        let timestamp = r.read_i64_be()?;
        let salt = r.read_i64_be()?;
        let argument_signatures = decode_argument_signatures(r, version)?;
        let last_seen = decode_last_seen(r, version)?;
        Ok(Self {
            command,
            timestamp,
            salt,
            argument_signatures,
            last_seen,
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_string(&self.command)?;
        w.write_i64_be(self.timestamp)?;
        w.write_i64_be(self.salt)?;
        encode_argument_signatures(w, &self.argument_signatures, version, Self::NAME)?;
        encode_last_seen(w, &self.last_seen, version)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SChatAcknowledgement {
    pub offset: i32,
    pub previous_messages: PreviousMessages,
}

impl Packet for SChatAcknowledgement {
    const NAME: &'static str = "SChatAcknowledgement";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Serverbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_19_1 => 0x03,
        V1_21_2 => 0x04,
        V1_21_6 => 0x05,
        V26_1   => 0x06,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        if version.less_than(ProtocolVersion::V1_19_3) {
            let previous_messages = decode_previous_messages(r)?;
            return Ok(Self {
                offset: 0,
                previous_messages,
            });
        }
        let offset = r.read_var_int()?.0;
        Ok(Self {
            offset,
            previous_messages: PreviousMessages::default(),
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        if version.less_than(ProtocolVersion::V1_19_3) {
            return encode_previous_messages(w, &self.previous_messages);
        }
        w.write_var_int(&VarInt(self.offset))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::packets::round_trip;

    const SENDER_A: uuid::Uuid = uuid::Uuid::from_u128(0x0011_2233_4455_6677_8899_AABB_CCDD_EEFF);
    const SENDER_B: uuid::Uuid = uuid::Uuid::from_u128(0xFFEE_DDCC_BBAA_9988_7766_5544_3322_1100);

    fn encoded<P: Packet>(packet: &P, version: ProtocolVersion) -> Vec<u8> {
        let mut buf = Vec::new();
        packet.encode(&mut buf, version).unwrap();
        buf
    }

    fn decoded<P: Packet>(bytes: &[u8], version: ProtocolVersion) -> P {
        let mut r = bytes;
        let packet = P::decode(&mut r, version).unwrap();
        assert!(r.is_empty(), "{} left {} unread bytes", P::NAME, r.len());
        packet
    }

    fn concat(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    fn signature(fill: u8) -> Vec<u8> {
        vec![fill; MESSAGE_SIGNATURE_LEN]
    }

    fn previous_messages() -> PreviousMessages {
        PreviousMessages {
            seen: vec![PreviousMessage {
                sender: SENDER_A,
                signature: vec![0x09, 0x08],
            }],
            last_received: Some(PreviousMessage {
                sender: SENDER_B,
                signature: vec![],
            }),
        }
    }

    const PREVIOUS_MESSAGES_BYTES: &[u8] = &[
        0x01, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD,
        0xEE, 0xFF, 0x02, 0x09, 0x08, 0x01, 0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA, 0x99, 0x88, 0x77,
        0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x00, 0x00,
    ];

    const TIMESTAMP_BYTES: &[u8] = &[0x00, 0x00, 0x01, 0x8F, 0x00, 0x00, 0x00, 0x2A];
    const TIMESTAMP: i64 = 0x0000_018F_0000_002A;
    const SALT_BYTES: &[u8] = &[0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
    const SALT: i64 = i64::MIN + 1;

    fn last_seen() -> LastSeenMessages {
        LastSeenMessages {
            offset: 300,
            acknowledged: [0x01, 0x80, 0x0F],
            checksum: 0xC3,
        }
    }

    const LAST_SEEN_BYTES: &[u8] = &[0xAC, 0x02, 0x01, 0x80, 0x0F];

    #[test]
    fn test_system_chat_round_trip_json() {
        let pkt = CSystemChatMessage::from_json(r#"{"text":"Hello!"}"#, false);
        let decoded = round_trip(&pkt, ProtocolVersion::V1_19);
        assert_eq!(
            std::str::from_utf8(&decoded.content).unwrap(),
            r#"{"text":"Hello!"}"#
        );
        assert!(!decoded.overlay);
    }

    #[test]
    fn test_system_chat_round_trip_nbt() {
        let nbt_data = vec![0x0A, 0x00, 0x00, 0x08, 0x00, 0x04];
        let pkt = CSystemChatMessage {
            content: nbt_data.clone(),
            overlay: true,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.content, nbt_data);
        assert!(decoded.overlay);
    }

    #[test]
    fn test_system_chat_overlay_flag() {
        let pkt = CSystemChatMessage::from_json(r#"{"text":"Action bar"}"#, true);
        let decoded = round_trip(&pkt, ProtocolVersion::V1_19_4);
        assert!(decoded.overlay);
    }

    #[test]
    fn test_system_chat_nbt_content_stops_before_overlay() {
        let pkt = CSystemChatMessage::from_nbt(vec![0x0A, 0x00, 0x00, 0x01], false);
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_20_3).unwrap();
        assert_eq!(buf, vec![0x0A, 0x00, 0x00, 0x01, 0x00]);

        let decoded =
            CSystemChatMessage::decode(&mut buf.as_slice(), ProtocolVersion::V1_20_3).unwrap();
        assert_eq!(decoded.content, vec![0x0A, 0x00, 0x00, 0x01]);
        assert!(!decoded.overlay);
    }

    #[test]
    fn test_system_chat_nbt_empty_payload_is_rejected() {
        assert!(CSystemChatMessage::decode(&mut [].as_slice(), ProtocolVersion::V1_20_3).is_err());
    }

    #[test]
    fn test_legacy_chat_1_7() {
        let pkt = CChatMessageLegacy {
            content: r#"{"text":"Hello"}"#.to_string(),
            position: 1,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_7_2);
        assert_eq!(decoded.content, r#"{"text":"Hello"}"#);
        assert_eq!(decoded.position, 0);
    }

    #[test]
    fn test_legacy_chat_1_8() {
        let pkt = CChatMessageLegacy {
            content: r#"{"text":"Hello"}"#.to_string(),
            position: 2,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_8);
        assert_eq!(decoded.content, r#"{"text":"Hello"}"#);
        assert_eq!(decoded.position, 2);
    }

    #[test]
    fn test_legacy_chat_1_16() {
        let pkt = CChatMessageLegacy {
            content: r#"{"text":"Hello"}"#.to_string(),
            position: 1,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_16);
        assert_eq!(decoded.content, r#"{"text":"Hello"}"#);
        assert_eq!(decoded.position, 1);
    }

    #[test]
    fn test_chat_message_before_1_19_is_the_string_alone() {
        let pkt = SChatMessage {
            message: "hi".to_string(),
            timestamp: TIMESTAMP,
            signature: Some(vec![1]),
            ..SChatMessage::default()
        };
        for version in [
            ProtocolVersion::V1_7_2,
            ProtocolVersion::V1_12_2,
            ProtocolVersion::V1_18_2,
        ] {
            let bytes = encoded(&pkt, version);
            assert_eq!(bytes, [0x02, b'h', b'i']);
            let back: SChatMessage = decoded(&bytes, version);
            assert_eq!(
                back,
                SChatMessage {
                    message: "hi".to_string(),
                    ..SChatMessage::default()
                }
            );
        }
    }

    #[test]
    fn test_chat_message_1_19_golden_bytes() {
        let pkt = SChatMessage {
            message: "hi".to_string(),
            timestamp: TIMESTAMP,
            salt: SALT,
            signature: Some(vec![0x01, 0x02, 0x03]),
            signed_preview: true,
            ..SChatMessage::default()
        };
        let expected = concat(&[
            &[0x02, b'h', b'i'],
            TIMESTAMP_BYTES,
            SALT_BYTES,
            &[0x03, 0x01, 0x02, 0x03],
            &[0x01],
        ]);
        assert_eq!(encoded(&pkt, ProtocolVersion::V1_19), expected);
        assert_eq!(
            decoded::<SChatMessage>(&expected, ProtocolVersion::V1_19),
            pkt
        );
    }

    #[test]
    fn test_chat_message_1_19_1_golden_bytes() {
        let pkt = SChatMessage {
            message: "hi".to_string(),
            timestamp: TIMESTAMP,
            salt: SALT,
            signature: None,
            signed_preview: false,
            previous_messages: previous_messages(),
            ..SChatMessage::default()
        };
        let expected = concat(&[
            &[0x02, b'h', b'i'],
            TIMESTAMP_BYTES,
            SALT_BYTES,
            &[0x00],
            &[0x00],
            PREVIOUS_MESSAGES_BYTES,
        ]);
        assert_eq!(encoded(&pkt, ProtocolVersion::V1_19_1), expected);
        assert_eq!(
            decoded::<SChatMessage>(&expected, ProtocolVersion::V1_19_1),
            pkt
        );
    }

    #[test]
    fn test_chat_message_1_19_3_golden_bytes() {
        let pkt = SChatMessage {
            message: "hi".to_string(),
            timestamp: TIMESTAMP,
            salt: SALT,
            signature: Some(signature(0xAB)),
            last_seen: LastSeenMessages {
                checksum: 0,
                ..last_seen()
            },
            ..SChatMessage::default()
        };
        let expected = concat(&[
            &[0x02, b'h', b'i'],
            TIMESTAMP_BYTES,
            SALT_BYTES,
            &[0x01],
            &signature(0xAB),
            LAST_SEEN_BYTES,
        ]);
        for version in [
            ProtocolVersion::V1_19_3,
            ProtocolVersion::V1_20_5,
            ProtocolVersion::V1_21_4,
        ] {
            assert_eq!(encoded(&pkt, version), expected, "{version}");
            assert_eq!(
                decoded::<SChatMessage>(&expected, version),
                pkt,
                "{version}"
            );
        }
    }

    #[test]
    fn test_chat_message_1_21_5_appends_the_checksum() {
        let pkt = SChatMessage {
            message: "hi".to_string(),
            timestamp: TIMESTAMP,
            salt: SALT,
            signature: None,
            last_seen: last_seen(),
            ..SChatMessage::default()
        };
        let expected = concat(&[
            &[0x02, b'h', b'i'],
            TIMESTAMP_BYTES,
            SALT_BYTES,
            &[0x00],
            LAST_SEEN_BYTES,
            &[0xC3],
        ]);
        for version in [
            ProtocolVersion::V1_21_5,
            ProtocolVersion::V1_21_11,
            ProtocolVersion::V26_2,
        ] {
            assert_eq!(encoded(&pkt, version), expected, "{version}");
            assert_eq!(
                decoded::<SChatMessage>(&expected, version),
                pkt,
                "{version}"
            );
        }
    }

    #[test]
    fn test_chat_message_before_1_19_3_treats_an_empty_signature_as_unsigned() {
        let pkt = SChatMessage {
            message: "hi".to_string(),
            signature: Some(Vec::new()),
            ..SChatMessage::default()
        };
        let back = round_trip(&pkt, ProtocolVersion::V1_19);
        assert_eq!(back.signature, None);
    }

    #[test]
    fn test_chat_message_rejects_a_signature_that_is_not_256_bytes_from_1_19_3() {
        let pkt = SChatMessage {
            message: "hi".to_string(),
            signature: Some(vec![0; 255]),
            ..SChatMessage::default()
        };
        let mut buf = Vec::new();
        assert!(pkt.encode(&mut buf, ProtocolVersion::V1_19_3).is_err());
        assert!(
            round_trip(&pkt, ProtocolVersion::V1_19_1)
                .signature
                .is_some()
        );
    }

    #[test]
    fn test_modified_unsigned_message_re_encodes_with_its_trailer() {
        let original = SChatMessage {
            message: "hello".to_string(),
            timestamp: TIMESTAMP,
            salt: SALT,
            signature: None,
            last_seen: last_seen(),
            ..SChatMessage::default()
        };
        let wire = encoded(&original, ProtocolVersion::V1_21_11);
        let mut modified = decoded::<SChatMessage>(&wire, ProtocolVersion::V1_21_11);
        modified.message = "bye".to_string();
        let rewritten = encoded(&modified, ProtocolVersion::V1_21_11);
        assert_eq!(rewritten[0], 3);
        assert_eq!(&rewritten[1..4], b"bye");
        assert_eq!(&rewritten[4..], &wire[6..]);
    }

    fn argument_signatures(fixed: bool) -> Vec<ArgumentSignature> {
        vec![
            ArgumentSignature {
                name: "msg".to_string(),
                signature: if fixed {
                    signature(0x11)
                } else {
                    vec![0x11; 3]
                },
            },
            ArgumentSignature {
                name: "a".to_string(),
                signature: if fixed { signature(0x22) } else { Vec::new() },
            },
        ]
    }

    #[test]
    fn test_chat_command_1_19_golden_bytes() {
        let pkt = SChatCommand {
            command: "say hi".to_string(),
            timestamp: TIMESTAMP,
            salt: SALT,
            argument_signatures: argument_signatures(false),
            signed_preview: true,
            ..SChatCommand::default()
        };
        let expected = concat(&[
            &[0x06],
            b"say hi",
            TIMESTAMP_BYTES,
            SALT_BYTES,
            &[0x02, 0x03, b'm', b's', b'g', 0x03, 0x11, 0x11, 0x11],
            &[0x01, b'a', 0x00],
            &[0x01],
        ]);
        assert_eq!(encoded(&pkt, ProtocolVersion::V1_19), expected);
        assert_eq!(
            decoded::<SChatCommand>(&expected, ProtocolVersion::V1_19),
            pkt
        );
    }

    #[test]
    fn test_chat_command_1_19_1_golden_bytes() {
        let pkt = SChatCommand {
            command: "say hi".to_string(),
            timestamp: TIMESTAMP,
            salt: SALT,
            argument_signatures: argument_signatures(false),
            signed_preview: false,
            previous_messages: previous_messages(),
            ..SChatCommand::default()
        };
        let expected = concat(&[
            &[0x06],
            b"say hi",
            TIMESTAMP_BYTES,
            SALT_BYTES,
            &[0x02, 0x03, b'm', b's', b'g', 0x03, 0x11, 0x11, 0x11],
            &[0x01, b'a', 0x00],
            &[0x00],
            PREVIOUS_MESSAGES_BYTES,
        ]);
        assert_eq!(encoded(&pkt, ProtocolVersion::V1_19_1), expected);
        assert_eq!(
            decoded::<SChatCommand>(&expected, ProtocolVersion::V1_19_1),
            pkt
        );
    }

    #[test]
    fn test_chat_command_1_19_3_golden_bytes() {
        let pkt = SChatCommand {
            command: "say hi".to_string(),
            timestamp: TIMESTAMP,
            salt: SALT,
            argument_signatures: argument_signatures(true),
            last_seen: LastSeenMessages {
                checksum: 0,
                ..last_seen()
            },
            ..SChatCommand::default()
        };
        let expected = concat(&[
            &[0x06],
            b"say hi",
            TIMESTAMP_BYTES,
            SALT_BYTES,
            &[0x02, 0x03, b'm', b's', b'g'],
            &signature(0x11),
            &[0x01, b'a'],
            &signature(0x22),
            LAST_SEEN_BYTES,
        ]);
        for version in [ProtocolVersion::V1_19_3, ProtocolVersion::V1_20_3] {
            assert_eq!(encoded(&pkt, version), expected, "{version}");
            assert_eq!(
                decoded::<SChatCommand>(&expected, version),
                pkt,
                "{version}"
            );
        }
    }

    #[test]
    fn test_chat_command_from_1_20_5_is_the_command_alone() {
        let pkt = SChatCommand {
            command: "spawn".to_string(),
            timestamp: TIMESTAMP,
            argument_signatures: argument_signatures(true),
            ..SChatCommand::default()
        };
        for version in [
            ProtocolVersion::V1_20_5,
            ProtocolVersion::V1_21_5,
            ProtocolVersion::V26_2,
        ] {
            let bytes = encoded(&pkt, version);
            assert_eq!(bytes, concat(&[&[0x05], b"spawn"]), "{version}");
            assert_eq!(
                decoded::<SChatCommand>(&bytes, version),
                SChatCommand {
                    command: "spawn".to_string(),
                    ..SChatCommand::default()
                }
            );
        }
    }

    #[test]
    fn test_chat_command_rejects_a_short_argument_signature_from_1_19_3() {
        let pkt = SChatCommand {
            command: "msg a b".to_string(),
            argument_signatures: argument_signatures(false),
            ..SChatCommand::default()
        };
        let mut buf = Vec::new();
        assert!(pkt.encode(&mut buf, ProtocolVersion::V1_19_3).is_err());
    }

    #[test]
    fn test_signed_chat_command_golden_bytes() {
        let pkt = SChatCommandSigned {
            command: "say hi".to_string(),
            timestamp: TIMESTAMP,
            salt: SALT,
            argument_signatures: argument_signatures(true),
            last_seen: last_seen(),
        };
        let without_checksum = concat(&[
            &[0x06],
            b"say hi",
            TIMESTAMP_BYTES,
            SALT_BYTES,
            &[0x02, 0x03, b'm', b's', b'g'],
            &signature(0x11),
            &[0x01, b'a'],
            &signature(0x22),
            LAST_SEEN_BYTES,
        ]);
        let with_checksum = concat(&[&without_checksum, &[0xC3]]);

        for version in [ProtocolVersion::V1_20_5, ProtocolVersion::V1_21_4] {
            assert_eq!(encoded(&pkt, version), without_checksum, "{version}");
            let back: SChatCommandSigned = decoded(&without_checksum, version);
            assert_eq!(back.last_seen.checksum, 0);
            assert_eq!(back.argument_signatures, pkt.argument_signatures);
        }
        for version in [
            ProtocolVersion::V1_21_5,
            ProtocolVersion::V1_21_11,
            ProtocolVersion::V26_2,
        ] {
            assert_eq!(encoded(&pkt, version), with_checksum, "{version}");
            assert_eq!(
                decoded::<SChatCommandSigned>(&with_checksum, version),
                pkt,
                "{version}"
            );
        }
    }

    #[test]
    fn test_signed_chat_command_without_arguments() {
        let pkt = SChatCommandSigned {
            command: "help".to_string(),
            ..SChatCommandSigned::default()
        };
        let bytes = encoded(&pkt, ProtocolVersion::V1_21_6);
        assert_eq!(
            bytes,
            concat(&[
                &[0x04],
                b"help",
                &[0; 8],
                &[0; 8],
                &[0x00],
                &[0x00, 0x00, 0x00, 0x00],
                &[0x00],
            ])
        );
        assert_eq!(
            decoded::<SChatCommandSigned>(&bytes, ProtocolVersion::V1_21_6),
            pkt
        );
    }

    #[test]
    fn test_chat_acknowledgement_1_19_1_carries_previous_messages() {
        let pkt = SChatAcknowledgement {
            offset: 0,
            previous_messages: previous_messages(),
        };
        assert_eq!(
            encoded(&pkt, ProtocolVersion::V1_19_1),
            PREVIOUS_MESSAGES_BYTES
        );
        assert_eq!(
            decoded::<SChatAcknowledgement>(PREVIOUS_MESSAGES_BYTES, ProtocolVersion::V1_19_1),
            pkt
        );
    }

    #[test]
    fn test_chat_acknowledgement_from_1_19_3_is_a_varint_offset() {
        let pkt = SChatAcknowledgement {
            offset: 300,
            ..SChatAcknowledgement::default()
        };
        for version in [
            ProtocolVersion::V1_19_3,
            ProtocolVersion::V1_21_2,
            ProtocolVersion::V1_21_5,
            ProtocolVersion::V26_2,
        ] {
            assert_eq!(encoded(&pkt, version), [0xAC, 0x02], "{version}");
            assert_eq!(decoded::<SChatAcknowledgement>(&[0xAC, 0x02], version), pkt);
        }
    }

    #[test]
    fn test_negative_counts_are_rejected() {
        let negative = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
        assert!(
            SChatAcknowledgement::decode(&mut negative.as_slice(), ProtocolVersion::V1_19_1)
                .is_err()
        );
        let command = concat(&[&[0x01, b'x'], &[0; 16], &negative]);
        assert!(SChatCommand::decode(&mut command.as_slice(), ProtocolVersion::V1_19_3).is_err());
    }

    #[test]
    fn test_huge_prefixed_signature_length_is_rejected_without_allocating() {
        let bytes = concat(&[
            &[0x01, b'x'],
            &[0; 16],
            &[0xFF, 0xFF, 0xFF, 0xFF, 0x07],
            &[0x00],
        ]);
        assert!(SChatMessage::decode(&mut bytes.as_slice(), ProtocolVersion::V1_19).is_err());
    }

    #[test]
    fn test_every_strict_prefix_fails_to_decode() {
        fn check<P: Packet>(packet: &P, version: ProtocolVersion) {
            let bytes = encoded(packet, version);
            for len in 0..bytes.len() {
                assert!(
                    P::decode(&mut &bytes[..len], version).is_err(),
                    "{} at {version} decoded a {len}-byte prefix of {}",
                    P::NAME,
                    bytes.len()
                );
            }
        }

        let message = SChatMessage {
            message: "hi".to_string(),
            timestamp: TIMESTAMP,
            salt: SALT,
            signature: Some(signature(0x5A)),
            signed_preview: false,
            previous_messages: previous_messages(),
            last_seen: last_seen(),
        };
        let unsigned_1_19 = SChatMessage {
            signature: Some(vec![7; 4]),
            signed_preview: true,
            ..message.clone()
        };
        let command = SChatCommand {
            command: "say hi".to_string(),
            timestamp: TIMESTAMP,
            salt: SALT,
            argument_signatures: argument_signatures(true),
            previous_messages: previous_messages(),
            last_seen: last_seen(),
            ..SChatCommand::default()
        };
        let legacy_command = SChatCommand {
            argument_signatures: argument_signatures(false),
            ..command.clone()
        };
        let signed = SChatCommandSigned {
            command: "say hi".to_string(),
            timestamp: TIMESTAMP,
            salt: SALT,
            argument_signatures: argument_signatures(true),
            last_seen: last_seen(),
        };
        let ack = SChatAcknowledgement {
            offset: 300,
            previous_messages: previous_messages(),
        };

        for version in [ProtocolVersion::V1_19, ProtocolVersion::V1_19_1] {
            check(&unsigned_1_19, version);
            check(&legacy_command, version);
        }
        for version in [
            ProtocolVersion::V1_19_3,
            ProtocolVersion::V1_20_3,
            ProtocolVersion::V1_21_5,
            ProtocolVersion::V1_21_11,
        ] {
            check(&message, version);
        }
        for version in [ProtocolVersion::V1_19_3, ProtocolVersion::V1_20_3] {
            check(&command, version);
        }
        for version in [
            ProtocolVersion::V1_20_5,
            ProtocolVersion::V1_21_5,
            ProtocolVersion::V26_2,
        ] {
            check(&signed, version);
            check(&command, version);
        }
        for version in [ProtocolVersion::V1_19_1, ProtocolVersion::V1_19_3] {
            check(&ack, version);
        }
    }
}
