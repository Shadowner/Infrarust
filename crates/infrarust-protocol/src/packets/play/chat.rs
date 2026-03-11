use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// System chat message packet (Clientbound, 1.19+).
///
/// Used for system messages, proxy announcements, etc.
/// Replaces the older ChatMessage packet for non-player messages.
///
/// Content format:
/// - Pre-1.20.3: JSON text component (String)
/// - 1.20.3+: NBT compound (binary)
///
/// Stored as opaque bytes. For pre-1.20.3, the bytes are UTF-8 JSON.
/// For 1.20.3+, the bytes are raw NBT.
#[derive(Debug, Clone)]
pub struct CSystemChatMessage {
    pub content: Vec<u8>,
    /// If true, displayed in the action bar instead of the chat box.
    pub overlay: bool,
}

impl CSystemChatMessage {
    /// Creates a system chat message from a JSON text component string.
    pub fn from_json(json: &str, overlay: bool) -> Self {
        Self {
            content: json.as_bytes().to_vec(),
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
            // NBT content followed by overlay bool.
            // Read all remaining, last byte is overlay.
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
            w.write_bool(self.overlay)?;
        } else {
            w.write_all(&self.content)?;
            w.write_bool(self.overlay)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
}
