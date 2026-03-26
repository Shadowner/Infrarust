//! Set Default Spawn Position packet (Clientbound).
//!
//! Sets the compass target and world spawn location.

use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// Packs block coordinates into a 64-bit position value.
///
/// Format: X (26 bits) | Z (26 bits) | Y (12 bits)
pub fn pack_block_position(x: i32, y: i32, z: i32) -> i64 {
    ((x as i64 & 0x3FF_FFFF) << 38) | ((z as i64 & 0x3FF_FFFF) << 12) | (y as i64 & 0xFFF)
}

/// Set Default Spawn Position packet (Clientbound).
///
/// The `location` field is a packed block position (X/Y/Z in 64 bits).
///
/// Format changes:
/// - Pre-1.21.9: `location` (i64) + `angle` (f32)
/// - 1.21.9+: `dimension_name` (String) + `location` (i64) + `yaw` (f32) + `pitch` (f32)
#[derive(Debug, Clone)]
pub struct CSetDefaultSpawnPosition {
    /// Dimension name identifier (1.21.9+). Defaults to `minecraft:overworld`.
    pub dimension_name: String,
    /// Packed block position (see [`pack_block_position`]).
    pub location: i64,
    /// Yaw angle at spawn.
    pub yaw: f32,
    /// Pitch angle at spawn (1.21.9+).
    pub pitch: f32,
}

impl CSetDefaultSpawnPosition {
    /// Creates a spawn position at the given block coordinates in the overworld.
    pub fn at(x: i32, y: i32, z: i32, yaw: f32) -> Self {
        Self {
            dimension_name: "minecraft:overworld".to_string(),
            location: pack_block_position(x, y, z),
            yaw,
            pitch: 0.0,
        }
    }

    /// Creates a spawn position at the given block coordinates in a specific dimension.
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

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

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
        // Y=64 → bits 0..11 = 64
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
