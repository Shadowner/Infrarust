use std::io::Write;

use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::ProtocolResult;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

use super::super::{Packet, PacketMapping};

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
    pub tooltip: Option<String>,
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

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
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
                Some(r.read_string()?)
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
        _version: ProtocolVersion,
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
                    w.write_string(tooltip)?;
                }
                None => {
                    w.write_u8(0)?;
                }
            }
        }
        Ok(())
    }
}
