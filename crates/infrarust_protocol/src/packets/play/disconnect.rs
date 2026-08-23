use crate::error::ProtocolResult;
use crate::packets::play::common::{read_text_component, write_text_component};
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

#[derive(Debug, Clone)]
pub struct CDisconnect {
    pub reason: Vec<u8>,
}

impl CDisconnect {
    pub fn from_json(json: &str) -> Self {
        Self {
            reason: json.as_bytes().to_vec(),
        }
    }

    pub fn from_nbt(nbt: Vec<u8>) -> Self {
        Self { reason: nbt }
    }

    pub fn as_json(&self) -> Option<&str> {
        std::str::from_utf8(&self.reason).ok()
    }
}

impl Packet for CDisconnect {
    const NAME: &'static str = "CDisconnect";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_7_2  => 0x40,
        V1_9    => 0x1A,
        V1_13   => 0x1B,
        V1_14   => 0x1A,
        V1_15   => 0x1B,
        V1_16   => 0x1A,
        V1_16_2 => 0x19,
        V1_17   => 0x1A,
        V1_19   => 0x17,
        V1_19_1 => 0x19,
        V1_19_3 => 0x17,
        V1_19_4 => 0x1A,
        V1_20_2 => 0x1B,
        V1_20_5 => 0x1D,
        V1_21_5 => 0x1C,
        V1_21_9 => 0x20,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let reason = read_text_component(r, version, 0, Self::NAME)?;
        Ok(Self { reason })
    }

    fn encode(
        &self,
        w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        write_text_component(w, &self.reason, version, Self::NAME, "reason")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::packets::round_trip;

    #[test]
    fn test_disconnect_round_trip_json() {
        let pkt = CDisconnect::from_json(r#"{"text":"You are banned!"}"#);
        let decoded = round_trip(&pkt, ProtocolVersion::V1_19);
        assert_eq!(decoded.as_json(), Some(r#"{"text":"You are banned!"}"#));
    }

    #[test]
    fn test_disconnect_round_trip_nbt() {
        let nbt_data = vec![0x0A, 0x00, 0x00, 0x08, 0x00, 0x04, 0x74, 0x65, 0x78, 0x74];
        let pkt = CDisconnect {
            reason: nbt_data.clone(),
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_20_3);
        assert_eq!(decoded.reason, nbt_data);
    }
}
