use std::io::Write;

use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::ProtocolResult;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

use super::super::{Packet, PacketMapping};
use super::common::write_text_component;
use crate::nbt::skip_network_nbt;

#[derive(Debug, Clone)]
pub struct STabCompleteRequest {
    pub transaction_id: i32,
    pub text: String,
}

impl Packet for STabCompleteRequest {
    const NAME: &'static str = "STabCompleteRequest";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Serverbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_13   => 0x05,
        V1_14   => 0x06,
        V1_19   => 0x08,
        V1_19_1 => 0x09,
        V1_19_3 => 0x08,
        V1_19_4 => 0x09,
        V1_20_2 => 0x0A,
        V1_20_5 => 0x0B,
        V1_21_2 => 0x0D,
        V1_21_6 => 0x0E,
        V26_1   => 0x0F,
    ];

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let transaction_id = r.read_var_int()?.0;
        let text = r.read_string()?;
        Ok(Self {
            transaction_id,
            text,
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_var_int(&VarInt(self.transaction_id))?;
        w.write_string(&self.text)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CTabCompleteResponse {
    pub transaction_id: i32,
    pub start: i32,
    pub length: i32,
    pub matches: Vec<TabCompleteMatch>,
}

#[derive(Debug, Clone)]
pub struct TabCompleteMatch {
    pub text: String,
    pub tooltip: Option<Vec<u8>>,
}

impl Packet for CTabCompleteResponse {
    const NAME: &'static str = "CTabCompleteResponse";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_13   => 0x10,
        V1_15   => 0x11,
        V1_16   => 0x10,
        V1_16_2 => 0x0F,
        V1_17   => 0x11,
        V1_19   => 0x0E,
        V1_19_3 => 0x0D,
        V1_19_4 => 0x0F,
        V1_20_2 => 0x10,
        V1_21_5 => 0x0F,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let transaction_id = r.read_var_int()?.0;
        let start = r.read_var_int()?.0;
        let length = r.read_var_int()?.0;
        let count = r.read_var_int()?.0;
        if count < 0 {
            return Err(crate::error::ProtocolError::invalid("negative match count"));
        }
        let mut matches = Vec::with_capacity((count as usize).min(1024));
        for _ in 0..count {
            let text = r.read_string()?;
            let has_tooltip = r.read_u8()? != 0;
            let tooltip = if has_tooltip {
                Some(read_tooltip(r, version)?)
            } else {
                None
            };
            matches.push(TabCompleteMatch { text, tooltip });
        }
        Ok(Self {
            transaction_id,
            start,
            length,
            matches,
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_var_int(&VarInt(self.transaction_id))?;
        w.write_var_int(&VarInt(self.start))?;
        w.write_var_int(&VarInt(self.length))?;
        w.write_var_int(&VarInt(self.matches.len() as i32))?;
        for m in &self.matches {
            w.write_string(&m.text)?;
            match &m.tooltip {
                Some(tooltip) => {
                    w.write_u8(1)?;
                    write_text_component(w, tooltip, version, Self::NAME, "tooltip")?;
                }
                None => {
                    w.write_u8(0)?;
                }
            }
        }
        Ok(())
    }
}

fn read_tooltip(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Vec<u8>> {
    if version.less_than(ProtocolVersion::V1_20_3) {
        return Ok(r.read_string()?.into_bytes());
    }
    let start = *r;
    skip_network_nbt(r)?;
    Ok(start[..start.len() - r.len()].to_vec())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn round_trip(tooltip: Vec<u8>, version: ProtocolVersion) -> CTabCompleteResponse {
        let response = CTabCompleteResponse {
            transaction_id: 7,
            start: 3,
            length: 2,
            matches: vec![
                TabCompleteMatch {
                    text: "alpha".into(),
                    tooltip: Some(tooltip),
                },
                TabCompleteMatch {
                    text: "beta".into(),
                    tooltip: None,
                },
            ],
        };
        let mut buf = Vec::new();
        response.encode(&mut buf, version).unwrap();
        let mut slice = buf.as_slice();
        let decoded = CTabCompleteResponse::decode(&mut slice, version).unwrap();
        assert!(slice.is_empty());
        decoded
    }

    #[test]
    fn json_tooltips_round_trip_before_1_20_3() {
        let json = br#"{"text":"hint"}"#.to_vec();
        let decoded = round_trip(json.clone(), ProtocolVersion::V1_20_2);
        assert_eq!(decoded.matches[0].tooltip.as_deref(), Some(json.as_slice()));
        assert_eq!(decoded.matches[1].text, "beta");
    }

    #[test]
    fn nbt_tooltips_round_trip_from_1_20_3() {
        let nbt = vec![0x08, 0x00, 0x04, b'h', b'i', b'n', b't'];
        let decoded = round_trip(nbt.clone(), ProtocolVersion::V1_20_3);
        assert_eq!(decoded.matches[0].tooltip.as_deref(), Some(nbt.as_slice()));
        assert_eq!(decoded.matches[1].text, "beta");
        assert_eq!(decoded.matches[1].tooltip, None);
    }
}
