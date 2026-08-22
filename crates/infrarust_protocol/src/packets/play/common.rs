use std::io::Write;

use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::{ProtocolError, ProtocolResult};
use crate::version::ProtocolVersion;

const NBT_COMPONENT_VERSION: ProtocolVersion = ProtocolVersion::V1_20_3;

pub fn read_text_component(
    r: &mut &[u8],
    version: ProtocolVersion,
    trailer_len: usize,
    packet_name: &str,
) -> ProtocolResult<Vec<u8>> {
    if version.less_than(NBT_COMPONENT_VERSION) {
        return Ok(r.read_string()?.into_bytes());
    }
    if trailer_len == 0 {
        return r.read_remaining();
    }
    let payload = *r;
    let split = payload.len().checked_sub(trailer_len).ok_or_else(|| {
        ProtocolError::invalid(format!(
            "{packet_name}: payload too short for its trailing fields"
        ))
    })?;
    *r = &payload[split..];
    Ok(payload[..split].to_vec())
}

pub fn write_text_component(
    mut w: &mut (impl Write + ?Sized),
    text: &[u8],
    version: ProtocolVersion,
    packet_name: &str,
    field_name: &str,
) -> ProtocolResult<()> {
    if version.less_than(NBT_COMPONENT_VERSION) {
        let json = std::str::from_utf8(text).map_err(|_| {
            ProtocolError::invalid(format!(
                "{packet_name} {field_name} is not valid UTF-8 for JSON version"
            ))
        })?;
        w.write_string(json)?;
    } else {
        w.write_all(text)?;
    }
    Ok(())
}

pub fn decode_death_location(r: &mut &[u8]) -> ProtocolResult<(Option<String>, Option<i64>)> {
    if r.read_bool()? {
        let dim = r.read_string()?;
        let pos = r.read_i64_be()?;
        Ok((Some(dim), Some(pos)))
    } else {
        Ok((None, None))
    }
}

pub fn encode_death_location(
    mut w: &mut (impl Write + ?Sized),
    death_dimension: Option<&str>,
    death_position: Option<i64>,
) -> ProtocolResult<()> {
    if let (Some(dim), Some(pos)) = (death_dimension, death_position) {
        w.write_bool(true)?;
        w.write_string(dim)?;
        w.write_i64_be(pos)?;
    } else {
        w.write_bool(false)?;
    }
    Ok(())
}

pub fn decode_world_info(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<(i32, i32)> {
    let portal_cooldown = r.read_var_int()?.0;
    let sea_level = if version.no_less_than(ProtocolVersion::V1_21_2) {
        r.read_var_int()?.0
    } else {
        63
    };
    Ok((portal_cooldown, sea_level))
}

pub fn encode_world_info(
    mut w: &mut (impl Write + ?Sized),
    portal_cooldown: i32,
    sea_level: i32,
    version: ProtocolVersion,
) -> ProtocolResult<()> {
    w.write_var_int(&VarInt(portal_cooldown))?;
    if version.no_less_than(ProtocolVersion::V1_21_2) {
        w.write_var_int(&VarInt(sea_level))?;
    }
    Ok(())
}
