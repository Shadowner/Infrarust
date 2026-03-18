//! Server switch Respawn construction.
//!
//! Builds a `CRespawn` packet for the server switch trick. The format changes
//! significantly across protocol versions, so the encoding is version-branched.
//! The constructed packet uses safe defaults appropriate for a server switch.

use crate::codec::{McBufWriteExt, VarInt};
use crate::error::ProtocolResult;
use crate::version::ProtocolVersion;

use super::dimension::DimensionInfo;
use super::respawn::CRespawn;

/// Creates a `CRespawn` packet for the server switch trick.
///
/// For pre-1.20.2: builds the version-specific wire format into `raw_payload`.
/// For 1.20.2+: constructs using struct fields (existing encode path).
///
/// Default values are safe for server switching:
/// - `gamemode`: 0 (survival) — the backend's JoinGame sets the real gamemode
/// - `hashed_seed`: 0 — client doesn't verify
/// - `is_debug` / `is_flat`: false
/// - `previous_gamemode`: -1 (none)
/// - `data_to_keep` / `copy_metadata`: keep all data
/// - `difficulty`: 2 (normal) for pre-1.14
pub fn for_switch(dimension: &DimensionInfo, version: ProtocolVersion) -> CRespawn {
    if version.no_less_than(ProtocolVersion::V1_20_2) {
        // 1.20.2+: use struct fields, existing encode path handles it
        let (dim_id, level_name) = match dimension {
            DimensionInfo::Legacy(id) => (*id, "minecraft:overworld".to_string()),
            DimensionInfo::Named(name) => (0, name.clone()),
        };
        return CRespawn {
            dimension: dim_id,
            level_name,
            hashed_seed: 0,
            gamemode: 0,
            previous_gamemode: -1,
            is_debug: false,
            is_flat: false,
            data_to_keep: 0x01,
            death_dimension: None,
            death_position: None,
            portal_cooldown: 0,
            sea_level: 63,
            raw_payload: None,
        };
    }

    // Pre-1.20.2: build raw payload bytes
    let mut raw = Vec::with_capacity(64);
    encode_switch_respawn(&mut raw, dimension, version)
        .expect("respawn switch encoding should not fail with valid DimensionInfo");
    CRespawn {
        raw_payload: Some(raw),
        ..Default::default()
    }
}

/// Encodes the version-specific Respawn payload for server switch.
fn encode_switch_respawn(
    w: &mut Vec<u8>,
    dimension: &DimensionInfo,
    version: ProtocolVersion,
) -> ProtocolResult<()> {
    let pvn = version.0;

    if pvn < 477 {
        // Pre-1.14: dimension(i32) + difficulty(u8) + gamemode(u8) + level_type(String)
        let dim_id = dimension_as_i32(dimension);
        w.write_i32_be(dim_id)?;
        w.write_u8(2)?; // difficulty: normal
        w.write_u8(0)?; // gamemode: survival
        w.write_string("default")?; // level_type
    } else if pvn < 573 {
        // 1.14-1.14.4: dimension(i32) + gamemode(u8) + level_type(String)
        let dim_id = dimension_as_i32(dimension);
        w.write_i32_be(dim_id)?;
        w.write_u8(0)?; // gamemode
        w.write_string("default")?;
    } else if pvn < 735 {
        // 1.15-1.15.2: dimension(i32) + hashed_seed(i64) + gamemode(u8) + level_type(String)
        let dim_id = dimension_as_i32(dimension);
        w.write_i32_be(dim_id)?;
        w.write_i64_be(0)?; // hashed_seed
        w.write_u8(0)?; // gamemode
        w.write_string("default")?;
    } else if pvn < 751 {
        // 1.16-1.16.1: dimension_type(NBT Compound) + dimension_name(Identifier)
        //   + hashed_seed(i64) + gamemode(u8) + prev_gamemode(i8)
        //   + is_debug(bool) + is_flat(bool) + copy_metadata(bool)
        let dim_name = dimension_as_name(dimension);

        // Write a minimal NBT Compound for dimension_type
        write_minimal_dimension_nbt(w, &dim_name)?;

        w.write_string(&dim_name)?; // dimension_name
        w.write_i64_be(0)?; // hashed_seed
        w.write_u8(0)?; // gamemode
        w.write_i8(-1)?; // previous_gamemode
        w.write_bool(false)?; // is_debug
        w.write_bool(false)?; // is_flat
        w.write_bool(true)?; // copy_metadata
    } else if pvn < 759 {
        // 1.16.2-1.18.2: dimension_type(Identifier) + dimension_name(Identifier)
        //   + hashed_seed(i64) + gamemode(u8) + prev_gamemode(i8)
        //   + is_debug(bool) + is_flat(bool) + copy_metadata(bool)
        let dim_name = dimension_as_name(dimension);
        w.write_string(&dim_name)?; // dimension_type (Identifier)
        w.write_string(&dim_name)?; // dimension_name
        w.write_i64_be(0)?; // hashed_seed
        w.write_u8(0)?; // gamemode
        w.write_i8(-1)?; // previous_gamemode
        w.write_bool(false)?; // is_debug
        w.write_bool(false)?; // is_flat
        w.write_bool(true)?; // copy_metadata
    } else if pvn < 761 {
        // 1.19-1.19.2: + last_death_location(Optional)
        let dim_name = dimension_as_name(dimension);
        w.write_string(&dim_name)?;
        w.write_string(&dim_name)?;
        w.write_i64_be(0)?;
        w.write_u8(0)?;
        w.write_i8(-1)?;
        w.write_bool(false)?;
        w.write_bool(false)?;
        w.write_bool(true)?; // copy_metadata
        w.write_bool(false)?; // has_death_location = false
    } else if pvn < 762 {
        // 1.19.3: data_kept(u8) replaces copy_metadata(bool)
        let dim_name = dimension_as_name(dimension);
        w.write_string(&dim_name)?;
        w.write_string(&dim_name)?;
        w.write_i64_be(0)?;
        w.write_u8(0)?;
        w.write_i8(-1)?;
        w.write_bool(false)?;
        w.write_bool(false)?;
        w.write_u8(0x01)?; // data_kept: keep all
        w.write_bool(false)?; // has_death_location = false
    } else {
        // 1.19.4-1.20.1: + portal_cooldown(VarInt)
        let dim_name = dimension_as_name(dimension);
        w.write_string(&dim_name)?;
        w.write_string(&dim_name)?;
        w.write_i64_be(0)?;
        w.write_u8(0)?;
        w.write_i8(-1)?;
        w.write_bool(false)?;
        w.write_bool(false)?;
        w.write_u8(0x01)?; // data_kept
        w.write_bool(false)?; // has_death_location = false
        w.write_var_int(&VarInt(0))?; // portal_cooldown
    }

    Ok(())
}

/// Converts a `DimensionInfo` to an i32 dimension ID for pre-1.16.
fn dimension_as_i32(dim: &DimensionInfo) -> i32 {
    match dim {
        DimensionInfo::Legacy(id) => *id,
        DimensionInfo::Named(name) => match name.as_str() {
            "minecraft:the_nether" => -1,
            "minecraft:the_end" => 1,
            _ => 0, // overworld / unknown
        },
    }
}

/// Converts a `DimensionInfo` to a namespaced string for 1.16+.
fn dimension_as_name(dim: &DimensionInfo) -> String {
    match dim {
        DimensionInfo::Named(name) => name.clone(),
        DimensionInfo::Legacy(id) => match id {
            -1 => "minecraft:the_nether".to_string(),
            1 => "minecraft:the_end".to_string(),
            _ => "minecraft:overworld".to_string(),
        },
    }
}

/// Writes a minimal NBT Compound for dimension_type (1.16-1.16.1 Respawn).
///
/// The client uses the JoinGame's dimension info, not the Respawn's, so this
/// can be a bare-minimum valid compound.
fn write_minimal_dimension_nbt(w: &mut Vec<u8>, _dim_name: &str) -> ProtocolResult<()> {
    // TAG_Compound root with empty name
    w.push(0x0A); // TAG_Compound
    w.extend_from_slice(&0u16.to_be_bytes()); // empty root name

    // TAG_End — empty compound is valid
    w.push(0x00);
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_for_switch_pre_1_14() {
        let dim = DimensionInfo::Legacy(0);
        let respawn = for_switch(&dim, ProtocolVersion::V1_8);
        let raw = respawn.raw_payload.expect("should have raw_payload");
        // dimension(i32=0) + difficulty(u8=2) + gamemode(u8=0) + level_type(String="default")
        assert_eq!(&raw[0..4], &0i32.to_be_bytes()); // dimension
        assert_eq!(raw[4], 2); // difficulty
        assert_eq!(raw[5], 0); // gamemode
    }

    #[test]
    fn test_for_switch_pre_1_14_nether() {
        let dim = DimensionInfo::Legacy(-1);
        let respawn = for_switch(&dim, ProtocolVersion::V1_8);
        let raw = respawn.raw_payload.expect("should have raw_payload");
        assert_eq!(&raw[0..4], &(-1i32).to_be_bytes());
    }

    #[test]
    fn test_for_switch_1_14() {
        let dim = DimensionInfo::Legacy(0);
        let respawn = for_switch(&dim, ProtocolVersion::V1_14);
        let raw = respawn.raw_payload.expect("should have raw_payload");
        // dimension(i32=0) + gamemode(u8=0) + level_type(String="default") — no difficulty
        assert_eq!(&raw[0..4], &0i32.to_be_bytes());
        assert_eq!(raw[4], 0); // gamemode (no difficulty byte)
    }

    #[test]
    fn test_for_switch_1_15() {
        let dim = DimensionInfo::Legacy(1);
        let respawn = for_switch(&dim, ProtocolVersion::V1_15);
        let raw = respawn.raw_payload.expect("should have raw_payload");
        // dimension(i32=1) + hashed_seed(i64=0) + gamemode(u8=0) + level_type(String)
        assert_eq!(&raw[0..4], &1i32.to_be_bytes());
        assert_eq!(&raw[4..12], &0i64.to_be_bytes()); // hashed_seed
        assert_eq!(raw[12], 0); // gamemode
    }

    #[test]
    fn test_for_switch_1_16_2() {
        let dim = DimensionInfo::Named("minecraft:the_nether".to_string());
        let respawn = for_switch(&dim, ProtocolVersion::V1_16_2);
        let raw = respawn.raw_payload.expect("should have raw_payload");
        // Should start with dimension_type as Identifier (String)
        // VarInt length + "minecraft:the_nether"
        assert!(!raw.is_empty());
    }

    #[test]
    fn test_for_switch_1_19() {
        let dim = DimensionInfo::Named("minecraft:overworld".to_string());
        let respawn = for_switch(&dim, ProtocolVersion::V1_19);
        let raw = respawn.raw_payload.expect("should have raw_payload");
        assert!(!raw.is_empty());
    }

    #[test]
    fn test_for_switch_1_19_3() {
        let dim = DimensionInfo::Named("minecraft:overworld".to_string());
        let respawn = for_switch(&dim, ProtocolVersion::V1_19_3);
        let raw = respawn.raw_payload.expect("should have raw_payload");
        assert!(!raw.is_empty());
    }

    #[test]
    fn test_for_switch_1_19_4() {
        let dim = DimensionInfo::Named("minecraft:overworld".to_string());
        let respawn = for_switch(&dim, ProtocolVersion::V1_19_4);
        let raw = respawn.raw_payload.expect("should have raw_payload");
        assert!(!raw.is_empty());
    }

    #[test]
    fn test_for_switch_1_20_2() {
        let dim = DimensionInfo::Named("minecraft:overworld".to_string());
        let respawn = for_switch(&dim, ProtocolVersion::V1_20_2);
        // 1.20.2+ uses struct fields, not raw_payload
        assert!(respawn.raw_payload.is_none());
        assert_eq!(respawn.level_name, "minecraft:overworld");
        assert_eq!(respawn.data_to_keep, 0x01);
    }

    #[test]
    fn test_dimension_as_i32_conversions() {
        assert_eq!(dimension_as_i32(&DimensionInfo::Legacy(0)), 0);
        assert_eq!(dimension_as_i32(&DimensionInfo::Legacy(-1)), -1);
        assert_eq!(
            dimension_as_i32(&DimensionInfo::Named("minecraft:the_nether".to_string())),
            -1
        );
        assert_eq!(
            dimension_as_i32(&DimensionInfo::Named("minecraft:the_end".to_string())),
            1
        );
        assert_eq!(
            dimension_as_i32(&DimensionInfo::Named("minecraft:overworld".to_string())),
            0
        );
    }
}
