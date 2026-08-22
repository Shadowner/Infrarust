use crate::io::PacketFrame;
use crate::version::ProtocolVersion;
use bytes::Bytes;

fn chunk_data_packet_id(version: ProtocolVersion) -> i32 {
    match version {
        v if v.no_less_than(ProtocolVersion::V1_14) && v.less_than(ProtocolVersion::V1_15) => 0x21,
        v if v.no_less_than(ProtocolVersion::V1_15) && v.less_than(ProtocolVersion::V1_16) => 0x22,
        v if v.no_less_than(ProtocolVersion::V1_16) && v.less_than(ProtocolVersion::V1_16_2) => {
            0x21
        }
        v if v.no_less_than(ProtocolVersion::V1_16_2) && v.less_than(ProtocolVersion::V1_17) => {
            0x20
        }
        v if v.no_less_than(ProtocolVersion::V1_17) && v.less_than(ProtocolVersion::V1_18) => 0x22,
        v if v.no_less_than(ProtocolVersion::V1_18) && v.less_than(ProtocolVersion::V1_19) => 0x22,
        v if v.no_less_than(ProtocolVersion::V1_19) && v.less_than(ProtocolVersion::V1_19_1) => {
            0x1F
        }
        v if v.no_less_than(ProtocolVersion::V1_19_1) && v.less_than(ProtocolVersion::V1_19_3) => {
            0x21
        }
        v if v.no_less_than(ProtocolVersion::V1_19_3) && v.less_than(ProtocolVersion::V1_19_4) => {
            0x20
        }
        v if v.no_less_than(ProtocolVersion::V1_19_4) && v.less_than(ProtocolVersion::V1_20_2) => {
            0x24
        }
        v if v.no_less_than(ProtocolVersion::V1_20_2) && v.less_than(ProtocolVersion::V1_20_5) => {
            0x25
        }
        v if v.no_less_than(ProtocolVersion::V1_20_5) && v.less_than(ProtocolVersion::V1_21_2) => {
            0x27
        }
        v if v.no_less_than(ProtocolVersion::V1_21_2) && v.less_than(ProtocolVersion::V1_21_5) => {
            0x28
        }
        v if v.no_less_than(ProtocolVersion::V1_21_5) && v.less_than(ProtocolVersion::V1_21_9) => {
            0x27
        }
        v if v.no_less_than(ProtocolVersion::V1_21_9) => 0x2C,
        v if v.no_less_than(ProtocolVersion::V1_13) => 0x22,
        v if v.no_less_than(ProtocolVersion::V1_9) => 0x20,
        _ => 0x21,
    }
}

pub fn build_chunk_data_frame(
    chunk_x: i32,
    chunk_z: i32,
    num_sections: usize,
    version: ProtocolVersion,
) -> Result<PacketFrame, crate::error::ProtocolError> {
    let id = chunk_data_packet_id(version);
    let payload = build_chunk_data_payload(chunk_x, chunk_z, num_sections, version);
    Ok(PacketFrame::new(id, Bytes::from(payload)))
}

fn build_chunk_data_payload(
    chunk_x: i32,
    chunk_z: i32,
    num_sections: usize,
    version: ProtocolVersion,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(300);

    buf.extend_from_slice(&chunk_x.to_be_bytes());
    buf.extend_from_slice(&chunk_z.to_be_bytes());

    if version.less_than(ProtocolVersion::V1_14) {
        build_pre_1_14_empty_chunk(&mut buf, version);
        return buf;
    }

    let sections = encode_empty_chunk_sections(num_sections, version);
    encode_empty_heightmaps(&mut buf, version);
    write_varint(&mut buf, sections.len() as i32);
    buf.extend_from_slice(&sections);
    write_varint(&mut buf, 0);

    if version.no_less_than(ProtocolVersion::V1_18) {
        encode_light_data(&mut buf, num_sections, version);
    }

    buf
}

fn build_pre_1_14_empty_chunk(buf: &mut Vec<u8>, version: ProtocolVersion) {
    buf.push(1);

    if version.less_than(ProtocolVersion::V1_8) {
        buf.extend_from_slice(&0_u16.to_be_bytes());
        buf.extend_from_slice(&0_u16.to_be_bytes());
        let biome_data = [0u8; 256];
        let compressed = zlib_compress(&biome_data);
        #[allow(clippy::cast_possible_truncation)]
        buf.extend_from_slice(&(compressed.len() as i32).to_be_bytes());
        buf.extend_from_slice(&compressed);
    } else if version.less_than(ProtocolVersion::V1_9) {
        buf.extend_from_slice(&0_u16.to_be_bytes());
        write_varint(buf, 256);
        buf.extend_from_slice(&[0u8; 256]);
    } else {
        write_varint(buf, 0);
        write_varint(buf, 256);
        buf.extend_from_slice(&[0u8; 256]);
        write_varint(buf, 0);
    }
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

fn encode_empty_chunk_sections(num_sections: usize, version: ProtocolVersion) -> Vec<u8> {
    let mut buf = Vec::with_capacity(num_sections * 8);
    for _ in 0..num_sections {
        encode_empty_section(&mut buf, version);
    }
    buf
}

fn encode_empty_section(buf: &mut Vec<u8>, version: ProtocolVersion) {
    let needs_data_length = version.less_than(ProtocolVersion::V1_21_5);

    buf.extend_from_slice(&0_i16.to_be_bytes());
    buf.push(0);
    write_varint(buf, 0);
    if needs_data_length {
        write_varint(buf, 0);
    }

    buf.push(0);
    write_varint(buf, 0);
    if needs_data_length {
        write_varint(buf, 0);
    }
}

fn encode_empty_heightmaps(buf: &mut Vec<u8>, version: ProtocolVersion) {
    if version.less_than(ProtocolVersion::V1_21_5) {
        encode_empty_heightmaps_nbt(buf, version);
    } else {
        encode_empty_heightmaps_map(buf);
    }
}

fn encode_empty_heightmaps_nbt(buf: &mut Vec<u8>, version: ProtocolVersion) {
    buf.push(0x0A);
    if version.less_than(ProtocolVersion::V1_20_2) {
        buf.extend_from_slice(&0_u16.to_be_bytes());
    }
    encode_nbt_long_array(buf, "MOTION_BLOCKING", 37);
    encode_nbt_long_array(buf, "WORLD_SURFACE", 37);
    buf.push(0x00);
}

fn encode_empty_heightmaps_map(buf: &mut Vec<u8>) {
    write_varint(buf, 3);
    for index in [1, 4, 5] {
        write_varint(buf, index);
        write_varint(buf, 37);
        for _ in 0..37 {
            buf.extend_from_slice(&0_i64.to_be_bytes());
        }
    }
}

fn encode_nbt_long_array(buf: &mut Vec<u8>, name: &str, count: i32) {
    buf.push(0x0C);
    let name_bytes = name.as_bytes();
    buf.extend_from_slice(&(name_bytes.len() as u16).to_be_bytes());
    buf.extend_from_slice(name_bytes);
    buf.extend_from_slice(&count.to_be_bytes());
    for _ in 0..count {
        buf.extend_from_slice(&0_i64.to_be_bytes());
    }
}

fn encode_light_data(buf: &mut Vec<u8>, num_sections: usize, _version: ProtocolVersion) {
    let total_bits = num_sections + 2;
    let num_longs: usize = total_bits.div_ceil(64);
    let all_set: u64 = if total_bits >= 64 {
        u64::MAX
    } else {
        (1_u64 << total_bits) - 1
    };

    for _ in 0..2 {
        write_varint(buf, num_longs as i32);
        for _ in 0..num_longs {
            buf.extend_from_slice(&0_u64.to_be_bytes());
        }
    }

    for _ in 0..2 {
        write_varint(buf, num_longs as i32);
        buf.extend_from_slice(&all_set.to_be_bytes());
        for _ in 1..num_longs {
            buf.extend_from_slice(&0_u64.to_be_bytes());
        }
    }

    write_varint(buf, 0);
    write_varint(buf, 0);
}

pub fn write_varint(buf: &mut Vec<u8>, value: i32) {
    let mut val = value as u32;
    loop {
        if val & !0x7F == 0 {
            buf.push(val as u8);
            return;
        }
        buf.push((val & 0x7F | 0x80) as u8);
        val >>= 7;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn test_empty_section_pre_1_21_5() {
        let mut buf = Vec::new();
        encode_empty_section(&mut buf, ProtocolVersion::V1_21);
        assert_eq!(buf.len(), 8, "pre-1.21.5: empty section should be 8 bytes");
    }

    #[test]
    fn test_empty_section_1_21_5_plus() {
        let mut buf = Vec::new();
        encode_empty_section(&mut buf, ProtocolVersion::V1_21_5);
        assert_eq!(buf.len(), 6, "1.21.5+: empty section should be 6 bytes");
    }

    #[test]
    fn test_empty_chunk_16_sections_end() {
        let data = encode_empty_chunk_sections(16, ProtocolVersion::V1_21);
        assert_eq!(data.len(), 16 * 8, "16 sections * 8 bytes = 128");
    }

    #[test]
    fn test_empty_chunk_16_sections_end_1_21_5() {
        let data = encode_empty_chunk_sections(16, ProtocolVersion::V1_21_5);
        assert_eq!(data.len(), 16 * 6, "16 sections * 6 bytes = 96");
    }

    #[test]
    fn test_heightmap_nbt_network_format_1_20_2() {
        let mut buf = Vec::new();
        encode_empty_heightmaps_nbt(&mut buf, ProtocolVersion::V1_20_2);
        assert_eq!(buf[0], 0x0A, "must start with TAG_Compound");
        assert_eq!(
            buf[1], 0x0C,
            "1.20.2+ network NBT: no name bytes after TAG_Compound"
        );
    }

    #[test]
    fn test_heightmap_nbt_standard_format_pre_1_20_2() {
        let mut buf = Vec::new();
        encode_empty_heightmaps_nbt(&mut buf, ProtocolVersion::V1_19_4);
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
        let payload = build_chunk_data_payload(3, -7, 16, ProtocolVersion::V1_21);
        assert_eq!(&payload[0..4], &3_i32.to_be_bytes());
        assert_eq!(&payload[4..8], &(-7_i32).to_be_bytes());
    }

    #[test]
    fn test_pre_1_14_empty_chunk_1_8() {
        let frame = build_chunk_data_frame(0, 0, 16, ProtocolVersion::V1_8).unwrap();
        assert_eq!(frame.id, 0x21);
        assert_eq!(frame.payload.len(), 4 + 4 + 1 + 2 + 2 + 256);
    }

    #[test]
    fn test_pre_1_14_empty_chunk_1_9() {
        let frame = build_chunk_data_frame(0, 0, 16, ProtocolVersion::V1_9).unwrap();
        assert_eq!(frame.id, 0x20);
        assert_eq!(frame.payload.len(), 4 + 4 + 1 + 1 + 2 + 256 + 1);
    }

    #[test]
    fn test_pre_1_14_empty_chunk_1_12() {
        let frame = build_chunk_data_frame(0, 0, 16, ProtocolVersion::V1_12).unwrap();
        assert_eq!(frame.id, 0x20);
        assert_eq!(frame.payload.len(), 4 + 4 + 1 + 1 + 2 + 256 + 1);
    }

    #[test]
    fn test_pre_1_14_empty_chunk_1_13() {
        let frame = build_chunk_data_frame(0, 0, 16, ProtocolVersion::V1_13).unwrap();
        assert_eq!(frame.id, 0x22);
        assert_eq!(frame.payload.len(), 4 + 4 + 1 + 1 + 2 + 256 + 1);
    }

    #[test]
    fn test_pre_1_14_empty_chunk_1_7() {
        let frame = build_chunk_data_frame(0, 0, 16, ProtocolVersion::V1_7_2).unwrap();
        assert_eq!(frame.id, 0x21);
        let payload = frame.payload.as_ref();
        assert_eq!(&payload[0..4], &0_i32.to_be_bytes());
        assert_eq!(&payload[4..8], &0_i32.to_be_bytes());
        assert_eq!(payload[8], 1);
        assert!(payload.len() > 17);
    }

    #[test]
    fn test_chunk_packet_id_pre_1_14_versions() {
        assert_eq!(chunk_data_packet_id(ProtocolVersion::V1_7_2), 0x21);
        assert_eq!(chunk_data_packet_id(ProtocolVersion::V1_8), 0x21);
        assert_eq!(chunk_data_packet_id(ProtocolVersion::V1_9), 0x20);
        assert_eq!(chunk_data_packet_id(ProtocolVersion::V1_12), 0x20);
        assert_eq!(chunk_data_packet_id(ProtocolVersion::V1_13), 0x22);
    }
}
