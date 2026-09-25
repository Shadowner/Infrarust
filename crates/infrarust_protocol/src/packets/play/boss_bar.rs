use uuid::Uuid;

use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::{ProtocolError, ProtocolResult};
use crate::packets::play::common::{read_nested_text_component, write_text_component};
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

#[derive(Debug, Clone, PartialEq)]
pub enum BossBarAction {
    Add {
        title: Vec<u8>,
        health: f32,
        color: i32,
        division: i32,
        flags: u8,
    },
    Remove,
    UpdateHealth(f32),
    UpdateTitle(Vec<u8>),
    UpdateStyle {
        color: i32,
        division: i32,
    },
    UpdateFlags(u8),
}

impl BossBarAction {
    pub const fn id(&self) -> i32 {
        match self {
            Self::Add { .. } => 0,
            Self::Remove => 1,
            Self::UpdateHealth(_) => 2,
            Self::UpdateTitle(_) => 3,
            Self::UpdateStyle { .. } => 4,
            Self::UpdateFlags(_) => 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CBossBar {
    pub id: Uuid,
    pub action: BossBarAction,
}

impl Packet for CBossBar {
    const NAME: &'static str = "CBossBar";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const ENCODE_ONLY: bool = true;
    const IDS: &'static [PacketMapping] = ids![
        V1_9    => 0x0C,
        V1_15   => 0x0D,
        V1_16   => 0x0C,
        V1_17   => 0x0D,
        V1_19   => 0x0A,
        V1_19_4 => 0x0B,
        V1_20_2 => 0x0A,
        V1_21_5 => 0x09,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let id = r.read_uuid()?;
        let action = match r.read_var_int()?.0 {
            0 => BossBarAction::Add {
                title: read_nested_text_component(r, version)?,
                health: r.read_f32_be()?,
                color: r.read_var_int()?.0,
                division: r.read_var_int()?.0,
                flags: r.read_u8()?,
            },
            1 => BossBarAction::Remove,
            2 => BossBarAction::UpdateHealth(r.read_f32_be()?),
            3 => BossBarAction::UpdateTitle(read_nested_text_component(r, version)?),
            4 => BossBarAction::UpdateStyle {
                color: r.read_var_int()?.0,
                division: r.read_var_int()?.0,
            },
            5 => BossBarAction::UpdateFlags(r.read_u8()?),
            other => {
                return Err(ProtocolError::invalid(format!(
                    "CBossBar: unknown action {other}"
                )));
            }
        };
        Ok(Self { id, action })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_uuid(&self.id)?;
        w.write_var_int(&VarInt(self.action.id()))?;
        match &self.action {
            BossBarAction::Add {
                title,
                health,
                color,
                division,
                flags,
            } => {
                write_text_component(&mut w, title, version, Self::NAME, "title")?;
                w.write_f32_be(*health)?;
                w.write_var_int(&VarInt(*color))?;
                w.write_var_int(&VarInt(*division))?;
                w.write_u8(*flags)?;
            }
            BossBarAction::Remove => {}
            BossBarAction::UpdateHealth(health) => w.write_f32_be(*health)?,
            BossBarAction::UpdateTitle(title) => {
                write_text_component(&mut w, title, version, Self::NAME, "title")?;
            }
            BossBarAction::UpdateStyle { color, division } => {
                w.write_var_int(&VarInt(*color))?;
                w.write_var_int(&VarInt(*division))?;
            }
            BossBarAction::UpdateFlags(flags) => w.write_u8(*flags)?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::packets::round_trip;

    fn every_action(title: &[u8]) -> Vec<BossBarAction> {
        vec![
            BossBarAction::Add {
                title: title.to_vec(),
                health: 0.5,
                color: 2,
                division: 1,
                flags: 0x05,
            },
            BossBarAction::Remove,
            BossBarAction::UpdateHealth(0.25),
            BossBarAction::UpdateTitle(title.to_vec()),
            BossBarAction::UpdateStyle {
                color: 6,
                division: 4,
            },
            BossBarAction::UpdateFlags(0x02),
        ]
    }

    #[test]
    fn every_action_round_trips_with_json_titles() {
        for action in every_action(br#"{"text":"Boss"}"#) {
            let packet = CBossBar {
                id: Uuid::from_u128(7),
                action,
            };
            assert_eq!(round_trip(&packet, ProtocolVersion::V1_9), packet);
        }
    }

    #[test]
    fn every_action_round_trips_with_nbt_titles() {
        let title = [0x08, 0x00, 0x04, b'B', b'o', b's', b's'];
        for action in every_action(&title) {
            let packet = CBossBar {
                id: Uuid::from_u128(7),
                action,
            };
            assert_eq!(round_trip(&packet, ProtocolVersion::V1_21_5), packet);
        }
    }

    #[test]
    fn add_writes_the_fields_in_wire_order() {
        let packet = CBossBar {
            id: Uuid::nil(),
            action: BossBarAction::Add {
                title: br#""x""#.to_vec(),
                health: 1.0,
                color: 3,
                division: 2,
                flags: 1,
            },
        };
        let mut buf = Vec::new();
        packet.encode(&mut buf, ProtocolVersion::V1_12_2).unwrap();
        let mut expected = vec![0u8; 16];
        expected.extend_from_slice(&[0x00, 0x03, b'"', b'x', b'"']);
        expected.extend_from_slice(&1.0f32.to_be_bytes());
        expected.extend_from_slice(&[0x03, 0x02, 0x01]);
        assert_eq!(buf, expected);
    }
}
