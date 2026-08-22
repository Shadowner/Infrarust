use crate::codec::{McBufWriteExt, VarInt};
use crate::error::ProtocolResult;
use crate::version::ProtocolVersion;

use super::dimension::DimensionInfo;
use super::respawn::CRespawn;

pub fn for_switch(dimension: &DimensionInfo, version: ProtocolVersion) -> CRespawn {
    if version.no_less_than(ProtocolVersion::V1_20_2) {
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

    let mut raw = Vec::with_capacity(64);
    encode_switch_respawn(&mut raw, dimension, version)
        .expect("respawn switch encoding should not fail with valid DimensionInfo");
    CRespawn {
        raw_payload: Some(raw),
        ..Default::default()
    }
}

fn encode_switch_respawn(
    w: &mut Vec<u8>,
    dimension: &DimensionInfo,
    version: ProtocolVersion,
) -> ProtocolResult<()> {
    if version.less_than(ProtocolVersion::V1_14) {
        let dim_id = dimension_as_i32(dimension);
        w.write_i32_be(dim_id)?;
        w.write_u8(2)?;
        w.write_u8(0)?;
        w.write_string("default")?;
    } else if version.less_than(ProtocolVersion::V1_15) {
        let dim_id = dimension_as_i32(dimension);
        w.write_i32_be(dim_id)?;
        w.write_u8(0)?;
        w.write_string("default")?;
    } else if version.less_than(ProtocolVersion::V1_16) {
        let dim_id = dimension_as_i32(dimension);
        w.write_i32_be(dim_id)?;
        w.write_i64_be(0)?;
        w.write_u8(0)?;
        w.write_string("default")?;
    } else if version.less_than(ProtocolVersion::V1_16_2) {
        let dim_name = dimension_as_name(dimension);

        write_minimal_dimension_nbt(w)?;

        w.write_string(&dim_name)?;
        w.write_i64_be(0)?;
        w.write_u8(0)?;
        w.write_i8(-1)?;
        w.write_bool(false)?;
        w.write_bool(false)?;
        w.write_bool(true)?;
    } else if version.less_than(ProtocolVersion::V1_19) {
        let dim_name = dimension_as_name(dimension);
        w.write_string(&dim_name)?;
        w.write_string(&dim_name)?;
        w.write_i64_be(0)?;
        w.write_u8(0)?;
        w.write_i8(-1)?;
        w.write_bool(false)?;
        w.write_bool(false)?;
        w.write_bool(true)?;
    } else if version.less_than(ProtocolVersion::V1_19_3) {
        let dim_name = dimension_as_name(dimension);
        w.write_string(&dim_name)?;
        w.write_string(&dim_name)?;
        w.write_i64_be(0)?;
        w.write_u8(0)?;
        w.write_i8(-1)?;
        w.write_bool(false)?;
        w.write_bool(false)?;
        w.write_bool(true)?;
        w.write_bool(false)?;
    } else if version.less_than(ProtocolVersion::V1_19_4) {
        let dim_name = dimension_as_name(dimension);
        w.write_string(&dim_name)?;
        w.write_string(&dim_name)?;
        w.write_i64_be(0)?;
        w.write_u8(0)?;
        w.write_i8(-1)?;
        w.write_bool(false)?;
        w.write_bool(false)?;
        w.write_u8(0x01)?;
        w.write_bool(false)?;
    } else {
        let dim_name = dimension_as_name(dimension);
        w.write_string(&dim_name)?;
        w.write_string(&dim_name)?;
        w.write_i64_be(0)?;
        w.write_u8(0)?;
        w.write_i8(-1)?;
        w.write_bool(false)?;
        w.write_bool(false)?;
        w.write_u8(0x01)?;
        w.write_bool(false)?;
        w.write_var_int(&VarInt(0))?;
    }

    Ok(())
}

fn dimension_as_i32(dim: &DimensionInfo) -> i32 {
    match dim {
        DimensionInfo::Legacy(id) => *id,
        DimensionInfo::Named(name) => match name.as_str() {
            "minecraft:the_nether" => -1,
            "minecraft:the_end" => 1,
            _ => 0,
        },
    }
}

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

fn write_minimal_dimension_nbt(w: &mut Vec<u8>) -> ProtocolResult<()> {
    w.push(0x0A);
    w.extend_from_slice(&0u16.to_be_bytes());
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
        assert_eq!(&raw[0..4], &0i32.to_be_bytes());
        assert_eq!(raw[4], 2);
        assert_eq!(raw[5], 0);
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
        assert_eq!(&raw[0..4], &0i32.to_be_bytes());
        assert_eq!(raw[4], 0);
    }

    #[test]
    fn test_for_switch_1_15() {
        let dim = DimensionInfo::Legacy(1);
        let respawn = for_switch(&dim, ProtocolVersion::V1_15);
        let raw = respawn.raw_payload.expect("should have raw_payload");
        assert_eq!(&raw[0..4], &1i32.to_be_bytes());
        assert_eq!(&raw[4..12], &0i64.to_be_bytes());
        assert_eq!(raw[12], 0);
    }

    #[test]
    fn test_for_switch_1_16_2() {
        let dim = DimensionInfo::Named("minecraft:the_nether".to_string());
        let respawn = for_switch(&dim, ProtocolVersion::V1_16_2);
        let raw = respawn.raw_payload.expect("should have raw_payload");
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
