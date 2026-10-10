use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::{ProtocolError, ProtocolResult};
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

pub const PLAYER_INVENTORY_SLOTS: u16 = 46;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CSetContainerContent {
    pub window_id: u8,
    pub state_id: i32,
    pub empty_slots: u16,
}

impl CSetContainerContent {
    #[must_use]
    pub const fn cleared_player_inventory() -> Self {
        Self {
            window_id: 0,
            state_id: 0,
            empty_slots: PLAYER_INVENTORY_SLOTS,
        }
    }

    fn write_empty_slot(
        w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        if version.no_less_than(ProtocolVersion::V1_13) {
            w.write_u8(0)
        } else {
            w.write_i16_be(-1)
        }
    }

    fn read_empty_slot(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<()> {
        let empty = if version.no_less_than(ProtocolVersion::V1_13) {
            r.read_u8()? == 0
        } else {
            r.read_i16_be()? == -1
        };
        if empty {
            Ok(())
        } else {
            Err(ProtocolError::invalid(
                "CSetContainerContent only decodes empty slots",
            ))
        }
    }
}

impl Packet for CSetContainerContent {
    const NAME: &'static str = "CSetContainerContent";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const ENCODE_ONLY: bool = true;
    const IDS: &'static [PacketMapping] = ids![
        V1_7_2  => 0x30,
        V1_9    => 0x14,
        V1_13   => 0x15,
        V1_14   => 0x14,
        V1_15   => 0x15,
        V1_16   => 0x14,
        V1_16_2 => 0x13,
        V1_17   => 0x14,
        V1_19   => 0x11,
        V1_19_3 => 0x10,
        V1_19_4 => 0x12,
        V1_20_2 => 0x13,
        V1_21_5 => 0x12,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let window_id = r.read_u8()?;
        let stateful = version.no_less_than(ProtocolVersion::V1_17_1);
        let state_id = if stateful { r.read_var_int()?.0 } else { 0 };
        let empty_slots = if stateful {
            u16::try_from(r.read_var_int()?.0)
                .map_err(|_| ProtocolError::invalid("negative container slot count"))?
        } else {
            u16::try_from(r.read_i16_be()?)
                .map_err(|_| ProtocolError::invalid("negative container slot count"))?
        };
        for _ in 0..empty_slots {
            Self::read_empty_slot(r, version)?;
        }
        if stateful {
            Self::read_empty_slot(r, version)?;
        }
        Ok(Self {
            window_id,
            state_id,
            empty_slots,
        })
    }

    fn encode(
        &self,
        w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_u8(self.window_id)?;
        let stateful = version.no_less_than(ProtocolVersion::V1_17_1);
        if stateful {
            w.write_var_int(&VarInt(self.state_id))?;
            w.write_var_int(&VarInt(i32::from(self.empty_slots)))?;
        } else {
            w.write_i16_be(self.empty_slots.cast_signed())?;
        }
        for _ in 0..self.empty_slots {
            Self::write_empty_slot(w, version)?;
        }
        if stateful {
            Self::write_empty_slot(w, version)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn encoded(version: ProtocolVersion) -> Vec<u8> {
        let mut buf = Vec::new();
        CSetContainerContent::cleared_player_inventory()
            .encode(&mut buf, version)
            .unwrap();
        buf
    }

    #[test]
    fn modern_layout_carries_state_id_and_carried_item() {
        let buf = encoded(ProtocolVersion::V1_21);
        assert_eq!(buf.len(), 1 + 1 + 1 + 46 + 1);
        assert_eq!(&buf[..3], &[0, 0, 46]);
        assert!(buf[3..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn legacy_layout_uses_a_short_count() {
        let buf = encoded(ProtocolVersion::V1_16);
        assert_eq!(buf.len(), 1 + 2 + 46);
        assert_eq!(&buf[..3], &[0, 0, 46]);
        let old = encoded(ProtocolVersion::V1_8);
        assert_eq!(old.len(), 1 + 2 + 46 * 2);
        assert_eq!(&old[3..5], &[0xFF, 0xFF]);
    }

    #[test]
    fn state_id_arrives_in_1_17_1() {
        let first = encoded(ProtocolVersion::V1_17);
        assert_eq!(first.len(), 1 + 2 + 46);
        assert_eq!(&first[..3], &[0, 0, 46]);
        let stateful = encoded(ProtocolVersion::V1_17_1);
        assert_eq!(stateful.len(), 1 + 1 + 1 + 46 + 1);
        assert_eq!(&stateful[..3], &[0, 0, 46]);
    }

    #[test]
    fn round_trip_in_every_layout() {
        let packet = CSetContainerContent::cleared_player_inventory();
        for version in [
            ProtocolVersion::V1_8,
            ProtocolVersion::V1_13,
            ProtocolVersion::V1_16,
            ProtocolVersion::V1_17,
            ProtocolVersion::V1_17_1,
            ProtocolVersion::V1_21_9,
        ] {
            let buf = encoded(version);
            let decoded = CSetContainerContent::decode(&mut buf.as_slice(), version).unwrap();
            assert_eq!(decoded, packet, "{version}");
        }
    }

    #[test]
    fn a_filled_slot_is_refused() {
        let mut buf = encoded(ProtocolVersion::V1_21);
        buf[3] = 1;
        assert!(CSetContainerContent::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).is_err());
    }
}
