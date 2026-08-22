use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

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

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        if version.less_than(ProtocolVersion::V1_20_3) {
            let content = r.read_string()?.into_bytes();
            let overlay = r.read_bool()?;
            Ok(Self { content, overlay })
        } else {
            let remaining = r.read_remaining()?;
            if remaining.is_empty() {
                return Err(crate::error::ProtocolError::invalid(
                    "CSystemChatMessage: empty payload",
                ));
            }
            let overlay = remaining[remaining.len() - 1] != 0;
            let content = remaining[..remaining.len() - 1].to_vec();
            Ok(Self { content, overlay })
        }
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        if version.less_than(ProtocolVersion::V1_20_3) {
            let json = std::str::from_utf8(&self.content).map_err(|_| {
                crate::error::ProtocolError::invalid(
                    "CSystemChatMessage content is not valid UTF-8 for JSON version",
                )
            })?;
            w.write_string(json)?;
        } else {
            w.write_all(&self.content)?;
        }
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

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

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
        mut w: &mut (impl std::io::Write + ?Sized),
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

#[derive(Debug, Clone)]
pub struct SChatMessage {
    pub message: String,
    pub remaining: Vec<u8>,
}

impl Packet for SChatMessage {
    const NAME: &'static str = "SChatMessage";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Serverbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let message = r.read_string()?;
        let remaining = r.read_remaining()?;
        Ok(Self { message, remaining })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_string(&self.message)?;
        w.write_all(&self.remaining)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SChatCommand {
    pub command: String,
    pub remaining: Vec<u8>,
}

impl Packet for SChatCommand {
    const NAME: &'static str = "SChatCommand";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Serverbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let command = r.read_string()?;
        let remaining = r.read_remaining()?;
        Ok(Self { command, remaining })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_string(&self.command)?;
        w.write_all(&self.remaining)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn round_trip<P: Packet>(packet: &P, version: ProtocolVersion) -> P {
        let mut buf = Vec::new();
        packet.encode(&mut buf, version).unwrap();
        P::decode(&mut buf.as_slice(), version).unwrap()
    }

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
    fn test_chat_message_round_trip() {
        let pkt = SChatMessage {
            message: "Hello world!".to_string(),
            remaining: vec![],
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_19);
        assert_eq!(decoded.message, "Hello world!");
        assert!(decoded.remaining.is_empty());
    }

    #[test]
    fn test_chat_message_with_remaining_bytes() {
        let pkt = SChatMessage {
            message: "test".to_string(),
            remaining: vec![0x01, 0x02, 0x03, 0xAA, 0xBB],
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_19_4);
        assert_eq!(decoded.message, "test");
        assert_eq!(decoded.remaining, vec![0x01, 0x02, 0x03, 0xAA, 0xBB]);
    }

    #[test]
    fn test_chat_command_round_trip() {
        let pkt = SChatCommand {
            command: "gamemode creative".to_string(),
            remaining: vec![],
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_19);
        assert_eq!(decoded.command, "gamemode creative");
        assert!(decoded.remaining.is_empty());
    }

    #[test]
    fn test_chat_command_with_remaining_bytes() {
        let pkt = SChatCommand {
            command: "tp Player1".to_string(),
            remaining: vec![0xFF, 0x00, 0x42],
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.command, "tp Player1");
        assert_eq!(decoded.remaining, vec![0xFF, 0x00, 0x42]);
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
}
