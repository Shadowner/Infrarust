use crate::codec::McBufReadExt;
use crate::error::{ProtocolError, ProtocolResult};
use crate::nbt;
use crate::version::ProtocolVersion;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DimensionInfo {
    Legacy(i32),
    Named(String),
}

pub fn extract_dimension_from_join_game(
    mut raw_payload: &[u8],
    version: ProtocolVersion,
) -> ProtocolResult<DimensionInfo> {
    let r = &mut raw_payload;

    if version.no_less_than(ProtocolVersion::V1_20_2) {
        return Ok(DimensionInfo::Named("minecraft:overworld".to_string()));
    }

    if version.less_than(ProtocolVersion::V1_16) {
        let _gamemode = r.read_u8()?;
        let dimension = r.read_i32_be()?;
        return Ok(DimensionInfo::Legacy(dimension));
    }

    let _is_hardcore = r.read_bool()?;
    let _gamemode = r.read_u8()?;
    let _previous_gamemode = r.read_i8()?;

    let world_count = r.read_var_int()?.0;
    if world_count < 0 {
        return Err(ProtocolError::invalid("negative world count"));
    }
    for _ in 0..world_count {
        let _world_name = r.read_string()?;
    }

    nbt::skip_nbt_compound(r)?;

    if version.less_than(ProtocolVersion::V1_16_2) {
        nbt::skip_nbt_compound(r)?;
    } else {
        let _dimension_type = r.read_string()?;
    }

    let dimension_name = r.read_string()?;
    Ok(DimensionInfo::Named(dimension_name))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::codec::McBufWriteExt;
    use crate::codec::VarInt;

    fn build_pre_1_16_payload(gamemode: u8, dimension: i32) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(gamemode);
        buf.extend_from_slice(&dimension.to_be_bytes());
        buf.extend_from_slice(&[0x00; 10]);
        buf
    }

    fn build_nbt_compound(name: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(0x0A);
        buf.extend_from_slice(&(name.len() as u16).to_be_bytes());
        buf.extend_from_slice(name.as_bytes());
        buf.push(0x00);
        buf
    }

    fn build_1_16_2_payload(dim_name: &str) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();
        buf.write_bool(false).unwrap();
        buf.push(0);
        buf.write_i8(-1).unwrap();

        buf.write_var_int(&VarInt(2)).unwrap();
        buf.write_string("minecraft:overworld").unwrap();
        buf.write_string("minecraft:the_nether").unwrap();

        buf.extend_from_slice(&build_nbt_compound(""));

        buf.write_string("minecraft:overworld").unwrap();

        buf.write_string(dim_name).unwrap();

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
        let dim = extract_dimension_from_join_game(&payload, ProtocolVersion::V1_16_2).unwrap();
        assert_eq!(
            dim,
            DimensionInfo::Named("minecraft:the_nether".to_string())
        );
    }

    #[test]
    fn test_extract_1_16_2_overworld() {
        let payload = build_1_16_2_payload("minecraft:overworld");
        let dim = extract_dimension_from_join_game(&payload, ProtocolVersion::V1_19).unwrap();
        assert_eq!(dim, DimensionInfo::Named("minecraft:overworld".to_string()));
    }

    #[test]
    fn test_extract_1_20_2_placeholder() {
        let dim = extract_dimension_from_join_game(&[], ProtocolVersion::V1_20_2).unwrap();
        assert_eq!(dim, DimensionInfo::Named("minecraft:overworld".to_string()));
    }

    fn build_1_16_2_header(world_count: i32) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();
        buf.write_bool(false).unwrap();
        buf.push(0);
        buf.write_i8(-1).unwrap();
        buf.write_var_int(&VarInt(world_count)).unwrap();
        buf
    }

    #[test]
    fn test_negative_world_count_rejected() {
        let buf = build_1_16_2_header(-1);
        let err = extract_dimension_from_join_game(&buf, ProtocolVersion::V1_16_2).unwrap_err();
        assert!(matches!(err, ProtocolError::Invalid { .. }));
    }

    #[test]
    fn test_hostile_world_count_errors() {
        let buf = build_1_16_2_header(i32::MAX);
        assert!(extract_dimension_from_join_game(&buf, ProtocolVersion::V1_16_2).is_err());
    }

    #[test]
    fn test_truncated_payload_errors() {
        let payload = build_1_16_2_payload("minecraft:overworld");
        let end = payload.len() - 10;
        for cut in 0..end {
            assert!(
                extract_dimension_from_join_game(&payload[..cut], ProtocolVersion::V1_16_2)
                    .is_err(),
                "prefix of {cut} bytes must error"
            );
        }
    }

    #[test]
    fn test_malformed_dimension_codec_errors() {
        let mut buf = build_1_16_2_header(0);
        buf.push(0x07);
        assert!(extract_dimension_from_join_game(&buf, ProtocolVersion::V1_16_2).is_err());
    }

    #[test]
    fn test_deeply_nested_dimension_codec_errors() {
        let mut nested = vec![0x01u8];
        nested.extend_from_slice(&0i32.to_be_bytes());
        for _ in 0..1024 {
            let mut outer = vec![0x09u8];
            outer.extend_from_slice(&1i32.to_be_bytes());
            outer.extend_from_slice(&nested);
            nested = outer;
        }
        let mut codec: Vec<u8> = vec![0x0A];
        codec.extend_from_slice(&0u16.to_be_bytes());
        codec.push(0x09);
        codec.extend_from_slice(&1u16.to_be_bytes());
        codec.push(b'x');
        codec.extend_from_slice(&nested);
        codec.push(0x00);

        let mut buf = build_1_16_2_header(0);
        buf.extend_from_slice(&codec);
        let result = extract_dimension_from_join_game(&buf, ProtocolVersion::V1_16_2);
        assert!(
            result.is_err(),
            "deeply nested codec must error, not overflow the stack"
        );
    }
}
