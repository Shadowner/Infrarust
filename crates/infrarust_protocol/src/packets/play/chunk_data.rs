use std::io::Write;

use crate::codec::McBufWriteExt;
use crate::codec::varint::VarInt;
use crate::error::{ProtocolError, ProtocolResult};
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

#[derive(Debug, Clone)]
pub struct CChunkData {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub num_sections: usize,
}

impl Packet for CChunkData {
    const NAME: &'static str = "CChunkData";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const ENCODE_ONLY: bool = true;
    const IDS: &'static [PacketMapping] = ids![
        V1_7_2  => 0x21,
        V1_9    => 0x20,
        V1_13   => 0x22,
        V1_14   => 0x21,
        V1_15   => 0x22,
        V1_16   => 0x21,
        V1_16_2 => 0x20,
        V1_17   => 0x22,
        V1_19   => 0x1F,
        V1_19_1 => 0x21,
        V1_19_3 => 0x20,
        V1_19_4 => 0x24,
        V1_20_2 => 0x25,
        V1_20_5 => 0x27,
        V1_21_2 => 0x28,
        V1_21_5 => 0x27,
        V1_21_9 => 0x2C,
    ];

    fn decode(_r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        Err(ProtocolError::invalid(
            "CChunkData is encode-only: the proxy never parses chunk data",
        ))
    }

    fn encode(
        &self,
        mut w: &mut (impl Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_i32_be(self.chunk_x)?;
        w.write_i32_be(self.chunk_z)?;

        if version.less_than(ProtocolVersion::V1_14) {
            return encode_pre_1_14_empty_chunk(&mut w, version);
        }

        let sections = encode_empty_chunk_sections(self.num_sections, version)?;
        encode_empty_heightmaps(&mut w, version)?;
        #[allow(clippy::cast_possible_truncation)]
        w.write_var_int(&VarInt(sections.len() as i32))?;
        w.write_all(&sections)?;
        w.write_var_int(&VarInt(0))?;

        if version.no_less_than(ProtocolVersion::V1_18) {
            encode_light_data(&mut w, self.num_sections)?;
        }

        Ok(())
    }
}

fn encode_pre_1_14_empty_chunk(w: &mut impl Write, version: ProtocolVersion) -> ProtocolResult<()> {
    w.write_u8(1)?;

    if version.less_than(ProtocolVersion::V1_8) {
        w.write_u16_be(0)?;
        w.write_u16_be(0)?;
        let compressed = zlib_compress(&[0u8; 256]);
        #[allow(clippy::cast_possible_truncation)]
        w.write_i32_be(compressed.len() as i32)?;
        w.write_all(&compressed)?;
    } else if version.less_than(ProtocolVersion::V1_9) {
        w.write_u16_be(0)?;
        w.write_var_int(&VarInt(256))?;
        w.write_all(&[0u8; 256])?;
    } else {
        w.write_var_int(&VarInt(0))?;
        w.write_var_int(&VarInt(256))?;
        w.write_all(&[0u8; 256])?;
        w.write_var_int(&VarInt(0))?;
    }

    Ok(())
}

fn zlib_compress(data: &[u8]) -> Vec<u8> {
    use crate::io::compression::new_compressor;

    let mut compressor = new_compressor(6);
    let mut out = Vec::new();
    compressor
        .compress(data, &mut out)
        .expect("zlib compression should not fail");
    out
}

fn encode_empty_chunk_sections(
    num_sections: usize,
    version: ProtocolVersion,
) -> ProtocolResult<Vec<u8>> {
    let mut buf = Vec::with_capacity(num_sections * 8);
    for _ in 0..num_sections {
        encode_empty_section(&mut buf, version)?;
    }
    Ok(buf)
}

fn encode_empty_section(w: &mut impl Write, version: ProtocolVersion) -> ProtocolResult<()> {
    let needs_data_length = version.less_than(ProtocolVersion::V1_21_5);

    w.write_i16_be(0)?;
    w.write_u8(0)?;
    w.write_var_int(&VarInt(0))?;
    if needs_data_length {
        w.write_var_int(&VarInt(0))?;
    }

    w.write_u8(0)?;
    w.write_var_int(&VarInt(0))?;
    if needs_data_length {
        w.write_var_int(&VarInt(0))?;
    }

    Ok(())
}

fn encode_empty_heightmaps(w: &mut impl Write, version: ProtocolVersion) -> ProtocolResult<()> {
    if version.less_than(ProtocolVersion::V1_21_5) {
        encode_empty_heightmaps_nbt(w, version)
    } else {
        encode_empty_heightmaps_map(w)
    }
}

fn encode_empty_heightmaps_nbt(w: &mut impl Write, version: ProtocolVersion) -> ProtocolResult<()> {
    w.write_u8(0x0A)?;
    if version.less_than(ProtocolVersion::V1_20_2) {
        w.write_u16_be(0)?;
    }
    encode_nbt_long_array(w, "MOTION_BLOCKING", 37)?;
    encode_nbt_long_array(w, "WORLD_SURFACE", 37)?;
    w.write_u8(0x00)?;
    Ok(())
}

fn encode_empty_heightmaps_map(w: &mut impl Write) -> ProtocolResult<()> {
    w.write_var_int(&VarInt(3))?;
    for index in [1, 4, 5] {
        w.write_var_int(&VarInt(index))?;
        w.write_var_int(&VarInt(37))?;
        for _ in 0..37 {
            w.write_i64_be(0)?;
        }
    }
    Ok(())
}

fn encode_nbt_long_array(w: &mut impl Write, name: &str, count: i32) -> ProtocolResult<()> {
    w.write_u8(0x0C)?;
    let name_bytes = name.as_bytes();
    #[allow(clippy::cast_possible_truncation)]
    w.write_u16_be(name_bytes.len() as u16)?;
    w.write_all(name_bytes)?;
    w.write_i32_be(count)?;
    for _ in 0..count {
        w.write_i64_be(0)?;
    }
    Ok(())
}

fn encode_light_data(w: &mut impl Write, num_sections: usize) -> ProtocolResult<()> {
    let total_bits = num_sections + 2;
    let num_longs: usize = total_bits.div_ceil(64);
    let all_set: u64 = if total_bits >= 64 {
        u64::MAX
    } else {
        (1_u64 << total_bits) - 1
    };

    for _ in 0..2 {
        #[allow(clippy::cast_possible_truncation)]
        w.write_var_int(&VarInt(num_longs as i32))?;
        for _ in 0..num_longs {
            w.write_u64_be(0)?;
        }
    }

    for _ in 0..2 {
        #[allow(clippy::cast_possible_truncation)]
        w.write_var_int(&VarInt(num_longs as i32))?;
        w.write_u64_be(all_set)?;
        for _ in 1..num_longs {
            w.write_u64_be(0)?;
        }
    }

    w.write_var_int(&VarInt(0))?;
    w.write_var_int(&VarInt(0))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::registry::build_default_registry;

    fn encode_payload(
        chunk_x: i32,
        chunk_z: i32,
        num_sections: usize,
        version: ProtocolVersion,
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        CChunkData {
            chunk_x,
            chunk_z,
            num_sections,
        }
        .encode(&mut buf, version)
        .unwrap();
        buf
    }

    fn packet_id(version: ProtocolVersion) -> i32 {
        build_default_registry()
            .get_packet_id::<CChunkData>(version)
            .unwrap()
    }

    #[test]
    fn decode_is_rejected() {
        assert!(CChunkData::decode(&mut [].as_slice(), ProtocolVersion::V1_21).is_err());
    }

    #[test]
    fn test_empty_section_pre_1_21_5() {
        let mut buf = Vec::new();
        encode_empty_section(&mut buf, ProtocolVersion::V1_21).unwrap();
        assert_eq!(buf.len(), 8, "pre-1.21.5: empty section should be 8 bytes");
    }

    #[test]
    fn test_empty_section_1_21_5_plus() {
        let mut buf = Vec::new();
        encode_empty_section(&mut buf, ProtocolVersion::V1_21_5).unwrap();
        assert_eq!(buf.len(), 6, "1.21.5+: empty section should be 6 bytes");
    }

    #[test]
    fn test_empty_chunk_16_sections_end() {
        let data = encode_empty_chunk_sections(16, ProtocolVersion::V1_21).unwrap();
        assert_eq!(data.len(), 16 * 8, "16 sections * 8 bytes = 128");
    }

    #[test]
    fn test_empty_chunk_16_sections_end_1_21_5() {
        let data = encode_empty_chunk_sections(16, ProtocolVersion::V1_21_5).unwrap();
        assert_eq!(data.len(), 16 * 6, "16 sections * 6 bytes = 96");
    }

    #[test]
    fn test_heightmap_nbt_network_format_1_20_2() {
        let mut buf = Vec::new();
        encode_empty_heightmaps_nbt(&mut buf, ProtocolVersion::V1_20_2).unwrap();
        assert_eq!(buf[0], 0x0A, "must start with TAG_Compound");
        assert_eq!(
            buf[1], 0x0C,
            "1.20.2+ network NBT: no name bytes after TAG_Compound"
        );
    }

    #[test]
    fn test_heightmap_nbt_standard_format_pre_1_20_2() {
        let mut buf = Vec::new();
        encode_empty_heightmaps_nbt(&mut buf, ProtocolVersion::V1_19_4).unwrap();
        assert_eq!(buf[0], 0x0A, "must start with TAG_Compound");
        assert_eq!(
            buf[1], 0x00,
            "pre-1.20.2 standard NBT: name length high byte"
        );
        assert_eq!(
            buf[2], 0x00,
            "pre-1.20.2 standard NBT: name length low byte"
        );
        assert_eq!(buf[3], 0x0C, "first inner tag after name");
    }

    #[test]
    fn test_chunk_data_payload_starts_correctly() {
        let payload = encode_payload(3, -7, 16, ProtocolVersion::V1_21);
        assert_eq!(&payload[0..4], &3_i32.to_be_bytes());
        assert_eq!(&payload[4..8], &(-7_i32).to_be_bytes());
    }

    #[test]
    fn test_pre_1_14_empty_chunk_1_8() {
        assert_eq!(packet_id(ProtocolVersion::V1_8), 0x21);
        let payload = encode_payload(0, 0, 16, ProtocolVersion::V1_8);
        assert_eq!(payload.len(), 4 + 4 + 1 + 2 + 2 + 256);
    }

    #[test]
    fn test_pre_1_14_empty_chunk_1_9() {
        assert_eq!(packet_id(ProtocolVersion::V1_9), 0x20);
        let payload = encode_payload(0, 0, 16, ProtocolVersion::V1_9);
        assert_eq!(payload.len(), 4 + 4 + 1 + 1 + 2 + 256 + 1);
    }

    #[test]
    fn test_pre_1_14_empty_chunk_1_12() {
        assert_eq!(packet_id(ProtocolVersion::V1_12), 0x20);
        let payload = encode_payload(0, 0, 16, ProtocolVersion::V1_12);
        assert_eq!(payload.len(), 4 + 4 + 1 + 1 + 2 + 256 + 1);
    }

    #[test]
    fn test_pre_1_14_empty_chunk_1_13() {
        assert_eq!(packet_id(ProtocolVersion::V1_13), 0x22);
        let payload = encode_payload(0, 0, 16, ProtocolVersion::V1_13);
        assert_eq!(payload.len(), 4 + 4 + 1 + 1 + 2 + 256 + 1);
    }

    #[test]
    fn test_pre_1_14_empty_chunk_1_7() {
        assert_eq!(packet_id(ProtocolVersion::V1_7_2), 0x21);
        let payload = encode_payload(0, 0, 16, ProtocolVersion::V1_7_2);
        assert_eq!(&payload[0..4], &0_i32.to_be_bytes());
        assert_eq!(&payload[4..8], &0_i32.to_be_bytes());
        assert_eq!(payload[8], 1, "ground_up_continuous");
        assert!(payload.len() > 17, "header plus compressed data");
    }

    #[test]
    fn test_chunk_packet_id_pre_1_14_versions() {
        assert_eq!(packet_id(ProtocolVersion::V1_7_2), 0x21);
        assert_eq!(packet_id(ProtocolVersion::V1_8), 0x21);
        assert_eq!(packet_id(ProtocolVersion::V1_9), 0x20);
        assert_eq!(packet_id(ProtocolVersion::V1_12), 0x20);
        assert_eq!(packet_id(ProtocolVersion::V1_13), 0x22);
    }
}
