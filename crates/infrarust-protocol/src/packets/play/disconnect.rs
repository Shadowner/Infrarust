use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// Play disconnect packet (Clientbound).
///
/// Sent by the server to kick a player with a reason message.
///
/// Format varies by version:
/// - Pre-1.20.3: `reason` is a JSON text component (String).
/// - 1.20.3+: `reason` is an NBT compound (binary).
///
/// The proxy stores the reason as opaque bytes to avoid parsing
/// the content format. Use the convenience methods for JSON construction.
#[derive(Debug, Clone)]
pub struct CDisconnect {
    pub reason: Vec<u8>,
}

impl CDisconnect {
    /// Creates a disconnect packet from a JSON text component string.
    pub fn from_json(json: &str) -> Self {
        Self {
            reason: json.as_bytes().to_vec(),
        }
    }

    /// Returns the reason as a JSON string, if the content is valid UTF-8.
    pub fn as_json(&self) -> Option<&str> {
        std::str::from_utf8(&self.reason).ok()
    }
}

impl Packet for CDisconnect {
    const NAME: &'static str = "CDisconnect";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let reason = if version.less_than(ProtocolVersion::V1_20_3) {
            r.read_string()?.into_bytes()
        } else {
            r.read_remaining()?
        };
        Ok(Self { reason })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        if version.less_than(ProtocolVersion::V1_20_3) {
            let json = std::str::from_utf8(&self.reason)
                .map_err(|_| crate::error::ProtocolError::invalid("CDisconnect reason is not valid UTF-8 for JSON version"))?;
            w.write_string(json)?;
        } else {
            w.write_all(&self.reason)?;
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
    fn test_disconnect_round_trip_json() {
        let pkt = CDisconnect::from_json(r#"{"text":"You are banned!"}"#);
        let decoded = round_trip(&pkt, ProtocolVersion::V1_19);
        assert_eq!(decoded.as_json(), Some(r#"{"text":"You are banned!"}"#));
    }

    #[test]
    fn test_disconnect_round_trip_nbt() {
        // Simulate NBT binary content
        let nbt_data = vec![0x0A, 0x00, 0x00, 0x08, 0x00, 0x04, 0x74, 0x65, 0x78, 0x74];
        let pkt = CDisconnect {
            reason: nbt_data.clone(),
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_20_3);
        assert_eq!(decoded.reason, nbt_data);
    }
}
