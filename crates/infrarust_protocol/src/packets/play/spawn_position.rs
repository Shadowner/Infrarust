use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

pub fn pack_block_position(x: i32, y: i32, z: i32) -> i64 {
    ((x as i64 & 0x3FF_FFFF) << 38) | ((z as i64 & 0x3FF_FFFF) << 12) | (y as i64 & 0xFFF)
}

#[derive(Debug, Clone)]
pub struct CSetDefaultSpawnPosition {
    pub dimension_name: String,
    pub location: i64,
    pub yaw: f32,
    pub pitch: f32,
}

impl CSetDefaultSpawnPosition {
    pub fn at(x: i32, y: i32, z: i32, yaw: f32) -> Self {
        Self {
            dimension_name: "minecraft:overworld".to_string(),
            location: pack_block_position(x, y, z),
            yaw,
            pitch: 0.0,
        }
    }

    pub fn at_in(dimension: &str, x: i32, y: i32, z: i32, yaw: f32) -> Self {
        Self {
            dimension_name: dimension.to_string(),
            location: pack_block_position(x, y, z),
            yaw,
            pitch: 0.0,
        }
    }
}

impl Packet for CSetDefaultSpawnPosition {
    const NAME: &'static str = "CSetDefaultSpawnPosition";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const ENCODE_ONLY: bool = true;
    const IDS: &'static [PacketMapping] = ids![
        V1_7_2  => 0x05,
        V1_9    => 0x43,
        V1_12   => 0x46,
        V1_13   => 0x49,
        V1_14   => 0x4D,
        V1_15   => 0x4E,
        V1_16   => 0x42,
        V1_17   => 0x4B,
        V1_18   => 0x4C,
        V1_19   => 0x4A,
        V1_19_1 => 0x4D,
        V1_19_3 => 0x4C,
        V1_19_4 => 0x50,
        V1_20_2 => 0x52,
        V1_20_3 => 0x54,
        V1_20_5 => 0x56,
        V1_21_2 => 0x5B,
        V1_21_5 => 0x5A,
        V1_21_9 => 0x5F,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let dimension_name = if version.no_less_than(ProtocolVersion::V1_21_9) {
            r.read_string()?
        } else {
            "minecraft:overworld".to_string()
        };
        let location = r.read_i64_be()?;
        let yaw = r.read_f32_be()?;
        let pitch = if version.no_less_than(ProtocolVersion::V1_21_9) {
            r.read_f32_be()?
        } else {
            0.0
        };
        Ok(Self {
            dimension_name,
            location,
            yaw,
            pitch,
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        if version.no_less_than(ProtocolVersion::V1_21_9) {
            w.write_string(&self.dimension_name)?;
        }
        w.write_i64_be(self.location)?;
        w.write_f32_be(self.yaw)?;
        if version.no_less_than(ProtocolVersion::V1_21_9) {
            w.write_f32_be(self.pitch)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn pack_origin() {
        let packed = pack_block_position(0, 64, 0);
        assert_eq!(packed & 0xFFF, 64);
    }

    #[test]
    fn round_trip() {
        let pkt = CSetDefaultSpawnPosition::at(0, 64, 0, 0.0);
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
        let decoded =
            CSetDefaultSpawnPosition::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(decoded.location, pkt.location);
        assert!((decoded.yaw - 0.0).abs() < f32::EPSILON);
    }
}
