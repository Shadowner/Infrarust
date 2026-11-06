use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

pub const ABILITY_INVULNERABLE: u8 = 0x01;
pub const ABILITY_FLYING: u8 = 0x02;
pub const ABILITY_MAY_FLY: u8 = 0x04;
pub const ABILITY_INSTABUILD: u8 = 0x08;

pub const DEFAULT_WALKING_SPEED: f32 = 0.1;

#[derive(Debug, Clone, PartialEq)]
pub struct CPlayerAbilities {
    pub flags: u8,
    pub flying_speed: f32,
    pub walking_speed: f32,
}

impl Packet for CPlayerAbilities {
    const NAME: &'static str = "CPlayerAbilities";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const ENCODE_ONLY: bool = true;
    const IDS: &'static [PacketMapping] = ids![
        V1_7_2  => 0x39,
        V1_9    => 0x2B,
        V1_12_1 => 0x2C,
        V1_13   => 0x2E,
        V1_14   => 0x31,
        V1_15   => 0x32,
        V1_16   => 0x31,
        V1_16_2 => 0x30,
        V1_17   => 0x32,
        V1_19   => 0x2F,
        V1_19_1 => 0x31,
        V1_19_3 => 0x30,
        V1_19_4 => 0x34,
        V1_20_2 => 0x36,
        V1_20_5 => 0x38,
        V1_21_2 => 0x3A,
        V1_21_5 => 0x39,
        V1_21_9 => 0x3E,
        V26_1   => 0x40,
        V26_3   => 0x41,
    ];

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        Ok(Self {
            flags: r.read_u8()?,
            flying_speed: r.read_f32_be()?,
            walking_speed: r.read_f32_be()?,
        })
    }

    fn encode(
        &self,
        w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_u8(self.flags)?;
        w.write_f32_be(self.flying_speed)?;
        w.write_f32_be(self.walking_speed)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn encodes_flags_then_both_speeds() {
        let pkt = CPlayerAbilities {
            flags: ABILITY_FLYING,
            flying_speed: 0.0,
            walking_speed: DEFAULT_WALKING_SPEED,
        };
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V26_3).unwrap();

        assert_eq!(buf[0], ABILITY_FLYING);
        assert_eq!(buf[1..5], 0.0_f32.to_be_bytes());
        assert_eq!(buf[5..], DEFAULT_WALKING_SPEED.to_be_bytes());
        assert_eq!(
            CPlayerAbilities::decode(&mut buf.as_slice(), ProtocolVersion::V26_3).unwrap(),
            pkt
        );
    }
}
