use crate::codec::{McBufWriteExt, VarInt};
use crate::error::ProtocolResult;
use crate::version::ProtocolVersion;

use super::dimension::DimensionInfo;
use super::respawn::CRespawn;

const DIFFICULTY_NORMAL: u8 = 2;
const GAMEMODE_SURVIVAL: u8 = 0;
const PREVIOUS_GAMEMODE_NONE: i8 = -1;
const HASHED_SEED: i64 = 0;
const LEVEL_TYPE_DEFAULT: &str = "default";
const IS_DEBUG: bool = false;
const IS_FLAT: bool = false;
const COPY_METADATA: bool = true;
const DATA_KEPT_ALL: u8 = 0x01;
const HAS_DEATH_LOCATION: bool = false;
const PORTAL_COOLDOWN: VarInt = VarInt(0);
const SEA_LEVEL: i32 = 63;

pub fn for_switch(dimension: &DimensionInfo, version: ProtocolVersion) -> CRespawn {
    if version.no_less_than(ProtocolVersion::V1_20_2) {
        let (dim_id, level_name) = match dimension {
            DimensionInfo::Legacy(id) => (*id, "minecraft:overworld".to_string()),
            DimensionInfo::Named(name) => (0, name.clone()),
        };
        return CRespawn {
            dimension: dim_id,
            level_name,
            hashed_seed: HASHED_SEED,
            gamemode: GAMEMODE_SURVIVAL,
            previous_gamemode: PREVIOUS_GAMEMODE_NONE,
            is_debug: IS_DEBUG,
            is_flat: IS_FLAT,
            data_to_keep: DATA_KEPT_ALL,
            death_dimension: None,
            death_position: None,
            portal_cooldown: PORTAL_COOLDOWN.0,
            sea_level: SEA_LEVEL,
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
    if version.less_than(ProtocolVersion::V1_16) {
        w.write_i32_be(dimension_as_i32(dimension))?;
        if version.less_than(ProtocolVersion::V1_14) {
            w.write_u8(DIFFICULTY_NORMAL)?;
        }
        if version.no_less_than(ProtocolVersion::V1_15) {
            w.write_i64_be(HASHED_SEED)?;
        }
        w.write_u8(GAMEMODE_SURVIVAL)?;
        return w.write_string(LEVEL_TYPE_DEFAULT);
    }

    let dim_name = dimension_as_name(dimension);
    if version.less_than(ProtocolVersion::V1_16_2) {
        write_minimal_dimension_nbt(w)?;
    } else {
        w.write_string(&dim_name)?;
    }
    w.write_string(&dim_name)?;
    w.write_i64_be(HASHED_SEED)?;
    w.write_u8(GAMEMODE_SURVIVAL)?;
    w.write_i8(PREVIOUS_GAMEMODE_NONE)?;
    w.write_bool(IS_DEBUG)?;
    w.write_bool(IS_FLAT)?;

    if version.less_than(ProtocolVersion::V1_19_3) {
        w.write_bool(COPY_METADATA)?;
    } else {
        w.write_u8(DATA_KEPT_ALL)?;
    }
    if version.no_less_than(ProtocolVersion::V1_19) {
        w.write_bool(HAS_DEATH_LOCATION)?;
    }
    if version.no_less_than(ProtocolVersion::V1_19_4) {
        w.write_var_int(&PORTAL_COOLDOWN)?;
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
    use std::fmt::Write as _;

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

    const GOLDEN_SWITCH_RESPAWN: &[(ProtocolVersion, &str, &str)] = &[
        (
            ProtocolVersion::V1_7_2,
            "ffffffff02000764656661756c74",
            "0000000102000764656661756c74",
        ),
        (
            ProtocolVersion::V1_7_6,
            "ffffffff02000764656661756c74",
            "0000000102000764656661756c74",
        ),
        (
            ProtocolVersion::V1_8,
            "ffffffff02000764656661756c74",
            "0000000102000764656661756c74",
        ),
        (
            ProtocolVersion::V1_9,
            "ffffffff02000764656661756c74",
            "0000000102000764656661756c74",
        ),
        (
            ProtocolVersion::V1_9_2,
            "ffffffff02000764656661756c74",
            "0000000102000764656661756c74",
        ),
        (
            ProtocolVersion::V1_9_4,
            "ffffffff02000764656661756c74",
            "0000000102000764656661756c74",
        ),
        (
            ProtocolVersion::V1_12,
            "ffffffff02000764656661756c74",
            "0000000102000764656661756c74",
        ),
        (
            ProtocolVersion::V1_12_1,
            "ffffffff02000764656661756c74",
            "0000000102000764656661756c74",
        ),
        (
            ProtocolVersion::V1_12_2,
            "ffffffff02000764656661756c74",
            "0000000102000764656661756c74",
        ),
        (
            ProtocolVersion::V1_13,
            "ffffffff02000764656661756c74",
            "0000000102000764656661756c74",
        ),
        (
            ProtocolVersion::V1_14,
            "ffffffff000764656661756c74",
            "00000001000764656661756c74",
        ),
        (
            ProtocolVersion::V1_15,
            "ffffffff0000000000000000000764656661756c74",
            "000000010000000000000000000764656661756c74",
        ),
        (
            ProtocolVersion::V1_16,
            "0a000000146d696e6563726166743a7468655f6e6574686572000000000000000000ff000001",
            "0a000000116d696e6563726166743a7468655f656e64000000000000000000ff000001",
        ),
        (
            ProtocolVersion::V1_16_2,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff000001",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff000001",
        ),
        (
            ProtocolVersion::V1_16_4,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff000001",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff000001",
        ),
        (
            ProtocolVersion::V1_17,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff000001",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff000001",
        ),
        (
            ProtocolVersion::V1_18,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff000001",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff000001",
        ),
        (
            ProtocolVersion::V1_18_2,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff000001",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff000001",
        ),
        (
            ProtocolVersion::V1_19,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff00000100",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff00000100",
        ),
        (
            ProtocolVersion::V1_19_1,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff00000100",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff00000100",
        ),
        (
            ProtocolVersion::V1_19_3,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff00000100",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff00000100",
        ),
        (
            ProtocolVersion::V1_19_4,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V1_20,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V1_20_2,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V1_20_3,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V1_20_5,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V1_21,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V1_21_2,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V1_21_4,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V1_21_5,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V1_21_6,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V1_21_7,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V1_21_9,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V1_21_11,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V26_1,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
        (
            ProtocolVersion::V26_2,
            "146d696e6563726166743a7468655f6e6574686572146d696e6563726166743a7468655f6e6574686572000000000000000000ff0000010000",
            "116d696e6563726166743a7468655f656e64116d696e6563726166743a7468655f656e64000000000000000000ff0000010000",
        ),
    ];

    fn to_hex(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            let _ = write!(out, "{b:02x}");
        }
        out
    }

    #[test]
    fn test_switch_respawn_golden_bytes_for_every_supported_version() {
        assert_eq!(
            GOLDEN_SWITCH_RESPAWN.len(),
            ProtocolVersion::SUPPORTED.len(),
            "golden table must cover every supported protocol version"
        );
        for ((version, named_hex, legacy_hex), supported) in
            GOLDEN_SWITCH_RESPAWN.iter().zip(ProtocolVersion::SUPPORTED)
        {
            assert_eq!(
                version, supported,
                "golden table must follow ProtocolVersion::SUPPORTED order"
            );
            let cases = [
                (
                    DimensionInfo::Named("minecraft:the_nether".to_string()),
                    *named_hex,
                ),
                (DimensionInfo::Legacy(1), *legacy_hex),
            ];
            for (dimension, expected) in cases {
                let mut buf = Vec::new();
                encode_switch_respawn(&mut buf, &dimension, *version)
                    .expect("switch respawn encoding must succeed");
                assert_eq!(
                    to_hex(&buf),
                    expected,
                    "payload changed for {version} / {dimension:?}"
                );
            }
        }
    }
}
