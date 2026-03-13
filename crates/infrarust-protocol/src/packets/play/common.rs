//! Shared helpers for play packets (`CJoinGame`, `CRespawn`).

use std::io::Write;

use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::ProtocolResult;
use crate::version::ProtocolVersion;

/// Decodes an optional death location (dimension identifier + packed position).
pub fn decode_death_location(
    r: &mut &[u8],
) -> ProtocolResult<(Option<String>, Option<i64>)> {
    if r.read_bool()? {
        let dim = r.read_string()?;
        let pos = r.read_i64_be()?;
        Ok((Some(dim), Some(pos)))
    } else {
        Ok((None, None))
    }
}

/// Encodes an optional death location (dimension identifier + packed position).
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

/// Decodes portal cooldown and sea level (version-dependent).
pub fn decode_world_info(
    r: &mut &[u8],
    version: ProtocolVersion,
) -> ProtocolResult<(i32, i32)> {
    let portal_cooldown = r.read_var_int()?.0;
    let sea_level = if version.no_less_than(ProtocolVersion::V1_21_2) {
        r.read_var_int()?.0
    } else {
        63
    };
    Ok((portal_cooldown, sea_level))
}

/// Encodes portal cooldown and sea level (version-dependent).
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
