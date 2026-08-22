use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
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
            let json = std::str::from_utf8(&self.reason).map_err(|_| {
                crate::error::ProtocolError::invalid(
                    "CDisconnect reason is not valid UTF-8 for JSON version",
                )
            })?;
            w.write_string(json)?;
        } else {
            w.write_all(&self.reason)?;
        }
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
