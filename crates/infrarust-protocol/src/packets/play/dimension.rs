//! Dimension extraction from JoinGame raw payload.
//!
//! The proxy needs to know the dimension from the JoinGame packet to construct
//! the correct Respawn packet during server switch (pre-1.20.2 only).

use crate::codec::McBufReadExt;
use crate::error::ProtocolResult;
use crate::nbt;
use crate::version::ProtocolVersion;

/// Dimension information extracted from a JoinGame packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DimensionInfo {
    /// Pre-1.16 dimension as integer: 0=overworld, -1=nether, 1=end.
    Legacy(i32),
    /// 1.16+ dimension as namespaced identifier: "minecraft:overworld".
    Named(String),
}

/// Extracts the dimension from a JoinGame packet's raw payload.
///
/// `raw_payload` is the `CJoinGame.raw_payload` field — everything AFTER `entity_id`
/// (which is parsed by `CJoinGame::decode`).
///
/// Returns a placeholder for 1.20.2+ (config phase flow, no Respawn trick needed).
///
/// # Errors
/// Returns `ProtocolError` if the payload is truncated or malformed.
pub fn extract_dimension_from_join_game(
    raw_payload: &[u8],
    version: ProtocolVersion,
) -> ProtocolResult<DimensionInfo> {
    let r = &mut raw_payload.as_ref();

    if version.no_less_than(ProtocolVersion::V1_20_2) {
        // 1.20.2+: config phase handles dimension, return placeholder
        return Ok(DimensionInfo::Named("minecraft:overworld".to_string()));
    }

    if version.less_than(ProtocolVersion::V1_16) {
        // Pre-1.16: [gamemode: u8] [dimension: i32] ...
        let _gamemode = r.read_u8()?;
        let dimension = r.read_i32_be()?;
        return Ok(DimensionInfo::Legacy(dimension));
    }

    // 1.16+: [is_hardcore: bool] [gamemode: u8] [prev_gamemode: i8]
    //         [world_count: VarInt] [world_names: String[]] [dimension_codec: NBT]
    //         [dimension_type: NBT(1.16-1.16.1) or Identifier(1.16.2+)]
    //         [dimension_name: Identifier] ...

    let _is_hardcore = r.read_bool()?;
    let _gamemode = r.read_u8()?;
    let _previous_gamemode = r.read_i8()?;

    // Skip world names array
    let world_count = r.read_var_int()?.0;
    for _ in 0..world_count {
        let _world_name = r.read_string()?;
    }

    // Skip dimension_codec (large NBT compound)
    nbt::skip_nbt_compound(r)?;

    if version.less_than(ProtocolVersion::V1_16_2) {
        // 1.16-1.16.1: dimension_type is an NBT Compound
        nbt::skip_nbt_compound(r)?;
    } else {
        // 1.16.2+: dimension_type is an Identifier (String)
        let _dimension_type = r.read_string()?;
    }

    // dimension_name is what we need
    let dimension_name = r.read_string()?;
    Ok(DimensionInfo::Named(dimension_name))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::codec::McBufWriteExt;
    use crate::codec::VarInt;

    /// Builds a pre-1.16 raw payload (after entity_id).
    fn build_pre_1_16_payload(gamemode: u8, dimension: i32) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(gamemode);
        buf.extend_from_slice(&dimension.to_be_bytes());
        // Remaining fields (max_players, etc.) — not parsed
        buf.extend_from_slice(&[0x00; 10]);
        buf
    }

    /// Builds a minimal NBT compound with a named root (for test payloads).
    fn build_nbt_compound(name: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(0x0A); // TAG_Compound
        buf.extend_from_slice(&(name.len() as u16).to_be_bytes());
        buf.extend_from_slice(name.as_bytes());
        buf.push(0x00); // TAG_End
        buf
    }

    /// Builds a 1.16.2+ raw payload (after entity_id).
    fn build_1_16_2_payload(dim_name: &str) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();
        buf.write_bool(false).unwrap(); // is_hardcore
        buf.push(0); // gamemode
        buf.write_i8(-1).unwrap(); // previous_gamemode

        // world_count + world_names
        buf.write_var_int(&VarInt(2)).unwrap();
        buf.write_string("minecraft:overworld").unwrap();
        buf.write_string("minecraft:the_nether").unwrap();

        // dimension_codec (NBT compound — minimal)
        buf.extend_from_slice(&build_nbt_compound(""));

        // dimension_type (Identifier for 1.16.2+)
        buf.write_string("minecraft:overworld").unwrap();

        // dimension_name (what we extract)
        buf.write_string(dim_name).unwrap();

        // Trailing data (hashed_seed etc.) — not parsed
        buf.extend_from_slice(&[0x00; 10]);
        buf
    }

    #[test]
    fn test_extract_pre_1_16_overworld() {
        let payload = build_pre_1_16_payload(1, 0);
        let dim = extract_dimension_from_join_game(&payload, ProtocolVersion::V1_8).unwrap();
        assert_eq!(dim, DimensionInfo::Legacy(0));
    }

    #[test]
    fn test_extract_pre_1_16_nether() {
        let payload = build_pre_1_16_payload(0, -1);
        let dim = extract_dimension_from_join_game(&payload, ProtocolVersion::V1_8).unwrap();
        assert_eq!(dim, DimensionInfo::Legacy(-1));
    }

    #[test]
    fn test_extract_pre_1_16_end() {
        let payload = build_pre_1_16_payload(0, 1);
        let dim = extract_dimension_from_join_game(&payload, ProtocolVersion::V1_15).unwrap();
        assert_eq!(dim, DimensionInfo::Legacy(1));
    }

    #[test]
    fn test_extract_1_16_2_named() {
        let payload = build_1_16_2_payload("minecraft:the_nether");
        let dim =
            extract_dimension_from_join_game(&payload, ProtocolVersion::V1_16_2).unwrap();
        assert_eq!(
            dim,
            DimensionInfo::Named("minecraft:the_nether".to_string())
        );
    }

    #[test]
    fn test_extract_1_16_2_overworld() {
        let payload = build_1_16_2_payload("minecraft:overworld");
        let dim =
            extract_dimension_from_join_game(&payload, ProtocolVersion::V1_19).unwrap();
        assert_eq!(
            dim,
            DimensionInfo::Named("minecraft:overworld".to_string())
        );
    }

    #[test]
    fn test_extract_1_20_2_placeholder() {
        let dim =
            extract_dimension_from_join_game(&[], ProtocolVersion::V1_20_2).unwrap();
        assert_eq!(
            dim,
            DimensionInfo::Named("minecraft:overworld".to_string())
        );
    }
}
