//! Empty chunk data encoder for Minecraft virtual worlds.
//!
//! Builds an all-air ChunkData packet as raw bytes in a [`PacketFrame`].
//! The ChunkData packet is never decoded by the proxy — we hardcode
//! the packet IDs per version instead.

use bytes::Bytes;
use crate::io::PacketFrame;
use crate::version::ProtocolVersion;

fn chunk_data_packet_id(version: ProtocolVersion) -> i32 {
    match version {
        // 1.14 (477) .. 1.14.4 (498)
        v if v.no_less_than(ProtocolVersion::V1_14)
            && v.less_than(ProtocolVersion::V1_15) =>
        {
            0x21
        }
        // 1.15 (573) .. 1.15.2 (578)
        v if v.no_less_than(ProtocolVersion::V1_15)
            && v.less_than(ProtocolVersion::V1_16) =>
        {
            0x22
        }
        // 1.16 (735) .. 1.16.1 (736)
        v if v.no_less_than(ProtocolVersion::V1_16)
            && v.less_than(ProtocolVersion::V1_16_2) =>
        {
            0x21
        }
        // 1.16.2 (751) .. 1.16.4 (754)
        v if v.no_less_than(ProtocolVersion::V1_16_2)
            && v.less_than(ProtocolVersion::V1_17) =>
        {
            0x20
        }
        // 1.17 (755) .. 1.17.1 (756)
        v if v.no_less_than(ProtocolVersion::V1_17)
            && v.less_than(ProtocolVersion::V1_18) =>
        {
            0x22
        }
        // 1.18 (757) .. 1.18.2 (758)
        v if v.no_less_than(ProtocolVersion::V1_18)
            && v.less_than(ProtocolVersion::V1_19) =>
        {
            0x22
        }
        // 1.19 (759)
        v if v.no_less_than(ProtocolVersion::V1_19)
            && v.less_than(ProtocolVersion::V1_19_1) =>
        {
            0x1F
        }
        // 1.19.1 (760) .. 1.19.2
        v if v.no_less_than(ProtocolVersion::V1_19_1)
            && v.less_than(ProtocolVersion::V1_19_3) =>
        {
            0x21
        }
        // 1.19.3 (761)
        v if v.no_less_than(ProtocolVersion::V1_19_3)
            && v.less_than(ProtocolVersion::V1_19_4) =>
        {
            0x20
        }
        // 1.19.4 (762) .. 1.20.1 (763)
        v if v.no_less_than(ProtocolVersion::V1_19_4)
            && v.less_than(ProtocolVersion::V1_20_2) =>
        {
            0x24
        }
        // 1.20.2 (764) .. 1.20.4 (765)
        v if v.no_less_than(ProtocolVersion::V1_20_2)
            && v.less_than(ProtocolVersion::V1_20_5) =>
        {
            0x25
        }
        // 1.20.5 (766) .. 1.21.1 (767)
        v if v.no_less_than(ProtocolVersion::V1_20_5)
            && v.less_than(ProtocolVersion::V1_21_2) =>
        {
            0x27
        }
        // 1.21.2 (768) .. 1.21.4 (769)
        v if v.no_less_than(ProtocolVersion::V1_21_2)
            && v.less_than(ProtocolVersion::V1_21_5) =>
        {
            0x28
        }
        // 1.21.5 (770) .. 1.21.7 (772)
        v if v.no_less_than(ProtocolVersion::V1_21_5)
            && v.less_than(ProtocolVersion::V1_21_9) =>
        {
            0x27
        }
        // 1.21.9 (773)+
        v if v.no_less_than(ProtocolVersion::V1_21_9) => 0x2C,
        // Fallback for older versions (pre-1.14)
        _ => 0x21,
    }
}

/// Builds a complete ChunkData packet for an all-air chunk.
pub fn build_chunk_data_frame(
    chunk_x: i32,
    chunk_z: i32,
    num_sections: usize,
    version: ProtocolVersion,
) -> Result<PacketFrame, crate::error::ProtocolError> {
    let id = chunk_data_packet_id(version);
    let payload = build_chunk_data_payload(chunk_x, chunk_z, num_sections, version);
    Ok(PacketFrame {
        id,
        payload: Bytes::from(payload),
    })
}

/// Wire layout: chunk X/Z (i32 BE), heightmaps, section data, block entities, light data (1.18+).
fn build_chunk_data_payload(chunk_x: i32, chunk_z: i32, num_sections: usize, version: ProtocolVersion) -> Vec<u8> {
    let sections = encode_empty_chunk_sections(num_sections, version);
    let mut buf = Vec::with_capacity(256 + sections.len());

    buf.extend_from_slice(&chunk_x.to_be_bytes());
    buf.extend_from_slice(&chunk_z.to_be_bytes());
    encode_empty_heightmaps(&mut buf, version);
    write_varint(&mut buf, sections.len() as i32);
    buf.extend_from_slice(&sections);
    write_varint(&mut buf, 0); // block_entities_count

    if version.no_less_than(ProtocolVersion::V1_18) {
        encode_light_data(&mut buf, num_sections, version);
    }

    buf
}

fn encode_empty_chunk_sections(num_sections: usize, version: ProtocolVersion) -> Vec<u8> {
    let mut buf = Vec::with_capacity(num_sections * 8);
    for _ in 0..num_sections {
        encode_empty_section(&mut buf, version);
    }
    buf
}

/// Pre-1.21.5: section has `VarInt(data_array_length)` after palette. 1.21.5+ omits it.
fn encode_empty_section(buf: &mut Vec<u8>, version: ProtocolVersion) {
    let needs_data_length = version.less_than(ProtocolVersion::V1_21_5);

    // Block states: count=0, bpe=0 (single-value), palette=air
    buf.extend_from_slice(&0_i16.to_be_bytes());
    buf.push(0);
    write_varint(buf, 0);
    if needs_data_length {
        write_varint(buf, 0);
    }

    // Biomes: bpe=0 (single-value), palette=plains
    buf.push(0);
    write_varint(buf, 0);
    if needs_data_length {
        write_varint(buf, 0);
    }
}

/// Pre-1.21.5: NBT compound. 1.21.5+: map format.
fn encode_empty_heightmaps(buf: &mut Vec<u8>, version: ProtocolVersion) {
    if version.less_than(ProtocolVersion::V1_21_5) {
        encode_empty_heightmaps_nbt(buf, version);
    } else {
        encode_empty_heightmaps_map(buf);
    }
}

fn encode_empty_heightmaps_nbt(buf: &mut Vec<u8>, version: ProtocolVersion) {
    buf.push(0x0A);                               // TAG_Compound
    if version.less_than(ProtocolVersion::V1_20_2) {
        buf.extend_from_slice(&0_u16.to_be_bytes()); // named root 
    }
    encode_nbt_long_array(buf, "MOTION_BLOCKING", 37);
    encode_nbt_long_array(buf, "WORLD_SURFACE", 37);
    buf.push(0x00);                                // TAG_End
}

/// Indices: 1=WORLD_SURFACE, 4=MOTION_BLOCKING, 5=MOTION_BLOCKING_NO_LEAVES.
fn encode_empty_heightmaps_map(buf: &mut Vec<u8>) {
    write_varint(buf, 3);
    for index in [1, 4, 5] {
        write_varint(buf, index);
        write_varint(buf, 37); // 37 longs per heightmap
        for _ in 0..37 {
            buf.extend_from_slice(&0_i64.to_be_bytes());
        }
    }
}

fn encode_nbt_long_array(buf: &mut Vec<u8>, name: &str, count: i32) {
    buf.push(0x0C); // TAG_Long_Array
    let name_bytes = name.as_bytes();
    buf.extend_from_slice(&(name_bytes.len() as u16).to_be_bytes());
    buf.extend_from_slice(name_bytes);
    buf.extend_from_slice(&count.to_be_bytes());
    for _ in 0..count {
        buf.extend_from_slice(&0_i64.to_be_bytes());
    }
}

/// All sections marked as empty light (no arrays). Both masks set, both array counts = 0.
fn encode_light_data(buf: &mut Vec<u8>, num_sections: usize, _version: ProtocolVersion) {
    let total_bits = num_sections + 2; // +2 for edge sections
    let num_longs: usize = (total_bits + 63) / 64;
    let all_set: u64 = if total_bits >= 64 { u64::MAX } else { (1_u64 << total_bits) - 1 };

    // sky_light_mask / block_light_mask: empty
    for _ in 0..2 {
        write_varint(buf, num_longs as i32);
        for _ in 0..num_longs {
            buf.extend_from_slice(&0_u64.to_be_bytes());
        }
    }

    // empty_sky_light_mask / empty_block_light_mask: all set
    for _ in 0..2 {
        write_varint(buf, num_longs as i32);
        buf.extend_from_slice(&all_set.to_be_bytes());
        for _ in 1..num_longs {
            buf.extend_from_slice(&0_u64.to_be_bytes());
        }
    }

    write_varint(buf, 0); // sky_light_arrays
    write_varint(buf, 0); // block_light_arrays
}

/// Encodes a VarInt directly into a `Vec<u8>`.
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
        assert_eq!(buf[1], 0x0C, "1.20.2+ network NBT: no name bytes after TAG_Compound");
    }

    #[test]
    fn test_heightmap_nbt_standard_format_pre_1_20_2() {
        let mut buf = Vec::new();
        encode_empty_heightmaps_nbt(&mut buf, ProtocolVersion::V1_19_4);
        assert_eq!(buf[0], 0x0A, "must start with TAG_Compound");
        assert_eq!(buf[1], 0x00, "pre-1.20.2 standard NBT: name length high byte");
        assert_eq!(buf[2], 0x00, "pre-1.20.2 standard NBT: name length low byte");
        assert_eq!(buf[3], 0x0C, "first inner tag after name");
    }

    #[test]
    fn test_chunk_data_payload_starts_correctly() {
        let payload = build_chunk_data_payload(3, -7, 16, ProtocolVersion::V1_21);
        assert_eq!(&payload[0..4], &3_i32.to_be_bytes());
        assert_eq!(&payload[4..8], &(-7_i32).to_be_bytes());
    }
}
