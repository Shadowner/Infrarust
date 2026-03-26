//! Synchronize Player Position packet (Clientbound, 1.9+).
//!
//! Teleports the player to a position. The client must respond with
//! Confirm Teleportation containing the matching `teleport_id`.

use crate::codec::varint::VarInt;
use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// Synchronize Player Position packet (Clientbound).
///
/// Also known as "Player Position And Look" in older protocol versions.
///
/// Format changes:
/// - Pre-1.9: x/y/z/yaw/pitch/flags (no teleport_id)
/// - 1.9–1.21.1: x/y/z/yaw/pitch/flags(u8)/teleport_id(VarInt)
/// - 1.21.2+: teleport_id(VarInt)/x/y/z/delta_x/delta_y/delta_z/yaw/pitch/flags(i32)
#[derive(Debug, Clone)]
pub struct CSynchronizePlayerPosition {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Velocity / delta (1.21.2+). Zero for static teleport.
    pub delta_x: f64,
    pub delta_y: f64,
    pub delta_z: f64,
    pub yaw: f32,
    pub pitch: f32,
    /// Relativity flags. For absolute positioning use 0.
    /// Stored as i32 for 1.21.2+ (u8 for older, widened on decode).
    pub flags: i32,
    /// Teleport ID — client echoes back in Confirm Teleportation (1.9+).
    pub teleport_id: i32,
}

impl Packet for CSynchronizePlayerPosition {
    const NAME: &'static str = "CSynchronizePlayerPosition";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        if version.no_less_than(ProtocolVersion::V1_21_2) {
            // 1.21.2+: teleport_id first, then position, delta, angles, flags(i32)
            let teleport_id = r.read_var_int()?.0;
            let x = r.read_f64_be()?;
            let y = r.read_f64_be()?;
            let z = r.read_f64_be()?;
            let delta_x = r.read_f64_be()?;
            let delta_y = r.read_f64_be()?;
            let delta_z = r.read_f64_be()?;
            let yaw = r.read_f32_be()?;
            let pitch = r.read_f32_be()?;
            let flags = r.read_i32_be()?;
            Ok(Self {
                x,
                y,
                z,
                delta_x,
                delta_y,
                delta_z,
                yaw,
                pitch,
                flags,
                teleport_id,
            })
        } else {
            // Pre-1.21.2: position, angles, flags(u8), teleport_id
            let x = r.read_f64_be()?;
            let y = r.read_f64_be()?;
            let z = r.read_f64_be()?;
            let yaw = r.read_f32_be()?;
            let pitch = r.read_f32_be()?;
            let flags = i32::from(r.read_u8()?);
            let teleport_id = if version.no_less_than(ProtocolVersion::V1_9) {
                r.read_var_int()?.0
            } else {
                0
            };
            Ok(Self {
                x,
                y,
                z,
                delta_x: 0.0,
                delta_y: 0.0,
                delta_z: 0.0,
                yaw,
                pitch,
                flags,
                teleport_id,
            })
        }
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        if version.no_less_than(ProtocolVersion::V1_21_2) {
            // 1.21.2+: teleport_id first
            w.write_var_int(&VarInt(self.teleport_id))?;
            w.write_f64_be(self.x)?;
            w.write_f64_be(self.y)?;
            w.write_f64_be(self.z)?;
            w.write_f64_be(self.delta_x)?;
            w.write_f64_be(self.delta_y)?;
            w.write_f64_be(self.delta_z)?;
            w.write_f32_be(self.yaw)?;
            w.write_f32_be(self.pitch)?;
            w.write_i32_be(self.flags)?;
        } else {
            w.write_f64_be(self.x)?;
            w.write_f64_be(self.y)?;
            w.write_f64_be(self.z)?;
            w.write_f32_be(self.yaw)?;
            w.write_f32_be(self.pitch)?;
            #[allow(clippy::cast_possible_truncation)]
            w.write_u8(self.flags as u8)?;
            if version.no_less_than(ProtocolVersion::V1_9) {
                w.write_var_int(&VarInt(self.teleport_id))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn round_trip() {
        let pkt = CSynchronizePlayerPosition {
            x: 0.0,
            y: 64.0,
            z: 0.0,
            delta_x: 0.0,
            delta_y: 0.0,
            delta_z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
            teleport_id: 0,
        };
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
        let decoded =
            CSynchronizePlayerPosition::decode(&mut buf.as_slice(), ProtocolVersion::V1_21)
                .unwrap();
        assert!((decoded.y - 64.0).abs() < f64::EPSILON);
        assert_eq!(decoded.teleport_id, 0);
        assert_eq!(decoded.flags, 0);
    }
}
