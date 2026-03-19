//! Empty chunk encoding for the Limbo world.
//!
//! Builds an all-air ChunkData packet as raw bytes in a [`PacketFrame`].
//! The ChunkData packet is never decoded by the proxy, so it is not registered
//! in the [`PacketRegistry`] — we hardcode the packet IDs per version instead.

use bytes::Bytes;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use crate::error::CoreError;

// ── Hardcoded ChunkData packet IDs (clientbound, Play state) ───────────

/// Returns the ChunkData (Level Chunk) packet ID for a given protocol version.
///
/// These are hardcoded because ChunkData is never decoded by the proxy,
/// so the packet is not registered in the [`PacketRegistry`].
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

// ── Public API ──────────────────────────────────────────────────────────

/// Builds a complete ChunkData packet for an all-air chunk, wrapped in a
/// [`PacketFrame`] ready for sending.
///
/// # Errors
/// Returns [`CoreError::Other`] if the version is unsupported or encoding fails.
pub(crate) fn build_chunk_data_frame(
    chunk_x: i32,
    chunk_z: i32,
    num_sections: usize,
    version: ProtocolVersion,
    _registry: &PacketRegistry,
) -> Result<PacketFrame, CoreError> {
    let id = chunk_data_packet_id(version);
    let payload = build_chunk_data_payload(chunk_x, chunk_z, num_sections, version);
    Ok(PacketFrame {
        id,
        payload: Bytes::from(payload),
    })
}

// ── Payload construction ────────────────────────────────────────────────

/// Builds the full ChunkData packet payload (everything after the packet ID).
///
/// Wire layout:
/// 1. `i32` BE — chunk X
/// 2. `i32` BE — chunk Z
/// 3. Heightmaps — NBT compound
/// 4. `VarInt` data length + section data
/// 5. `VarInt(0)` — block entities count
/// 6. Light data (1.18+ only, where light is bundled in ChunkData)
fn build_chunk_data_payload(chunk_x: i32, chunk_z: i32, num_sections: usize, version: ProtocolVersion) -> Vec<u8> {
    let sections = encode_empty_chunk_sections(num_sections, version);

    // Pre-allocate a reasonable buffer
    let mut buf = Vec::with_capacity(256 + sections.len());

    // 1. Chunk coordinates
    buf.extend_from_slice(&chunk_x.to_be_bytes());
    buf.extend_from_slice(&chunk_z.to_be_bytes());

    // 2. Heightmaps (format changed in 1.21.5)
    encode_empty_heightmaps(&mut buf, version);

    // 3. Chunk section data (prefixed by VarInt length)
    write_varint(&mut buf, sections.len() as i32);
    buf.extend_from_slice(&sections);

    // 4. Block entities: count = 0
    write_varint(&mut buf, 0);

    // 5. Light data (included in ChunkData since 1.18)
    if version.no_less_than(ProtocolVersion::V1_18) {
        encode_light_data(&mut buf, num_sections, version);
    }

    buf
}

// ── Section encoding ────────────────────────────────────────────────────

/// Encodes all chunk sections as empty (all-air).
///
/// Returns 24 sections for 1.18+ (protocol >= 757), 16 for older versions.
fn encode_empty_chunk_sections(num_sections: usize, version: ProtocolVersion) -> Vec<u8> {
    let mut buf = Vec::with_capacity(num_sections * 8);
    for _ in 0..num_sections {
        encode_empty_section(&mut buf, version);
    }
    buf
}

/// Encodes a single empty (all-air) chunk section.
///
/// Format changes at 1.21.5:
/// - **<= 1.21.4**: packed data uses `VarInt(len)` + `i64[]` (list-prefixed).
///   For single-valued palette, this means `VarInt(0)` (empty list).
/// - **>= 1.21.5**: packed data is written directly (no length prefix).
///   For single-valued palette, nothing is written.
fn encode_empty_section(buf: &mut Vec<u8>, version: ProtocolVersion) {
    let needs_data_length = version.less_than(ProtocolVersion::V1_21_5);

    // Block states
    buf.extend_from_slice(&0_i16.to_be_bytes()); // block_count = 0
    buf.push(0);                                  // bits_per_entry = 0 (single-value)
    write_varint(buf, 0);                         // palette_value = 0 (air)
    if needs_data_length {
        write_varint(buf, 0);                     // data_array_length = 0 (pre-1.21.5 only)
    }

    // Biomes
    buf.push(0);                                  // bits_per_entry = 0 (single-value)
    write_varint(buf, 0);                         // palette_value = 0 (plains)
    if needs_data_length {
        write_varint(buf, 0);                     // data_array_length = 0 (pre-1.21.5 only)
    }
}

// ── Heightmaps ──────────────────────────────────────────────────────────

/// Encodes the heightmaps for an all-air chunk.
///
/// Format changes at 1.21.5:
/// - **<= 1.21.4**: NBT compound with MOTION_BLOCKING + WORLD_SURFACE as TAG_Long_Array.
/// - **>= 1.21.5**: Map format with `VarInt(count)` + entries `(VarInt(index) + VarInt(len) + i64[])`.
fn encode_empty_heightmaps(buf: &mut Vec<u8>, version: ProtocolVersion) {
    if version.less_than(ProtocolVersion::V1_21_5) {
        encode_empty_heightmaps_nbt(buf);
    } else {
        encode_empty_heightmaps_map(buf);
    }
}

/// NBT format heightmaps (<= 1.21.4).
fn encode_empty_heightmaps_nbt(buf: &mut Vec<u8>) {
    // TAG_Compound (id = 0x0A) with empty name
    buf.push(0x0A);
    buf.extend_from_slice(&0_u16.to_be_bytes()); // name length = 0

    encode_nbt_long_array(buf, "MOTION_BLOCKING", 37);
    encode_nbt_long_array(buf, "WORLD_SURFACE", 37);

    buf.push(0x00); // TAG_End
}

/// Map format heightmaps (>= 1.21.5).
///
/// Format: `VarInt(3)` map size, then 3 entries:
/// - Index 1 = WORLD_SURFACE
/// - Index 4 = MOTION_BLOCKING
/// - Index 5 = MOTION_BLOCKING_NO_LEAVES
///
/// Each entry: `VarInt(index)` + `VarInt(37)` + 37 × `i64(0)`.
fn encode_empty_heightmaps_map(buf: &mut Vec<u8>) {
    write_varint(buf, 3); // map size = 3 entries

    for index in [1, 4, 5] {
        write_varint(buf, index);   // heightmap index
        write_varint(buf, 37);      // array length = 37 longs
        for _ in 0..37 {
            buf.extend_from_slice(&0_i64.to_be_bytes());
        }
    }
}

/// Writes an NBT TAG_Long_Array (id = 0x0C) with a given name and `count` zeros.
fn encode_nbt_long_array(buf: &mut Vec<u8>, name: &str, count: i32) {
    buf.push(0x0C); // TAG_Long_Array
    let name_bytes = name.as_bytes();
    buf.extend_from_slice(&(name_bytes.len() as u16).to_be_bytes());
    buf.extend_from_slice(name_bytes);
    buf.extend_from_slice(&count.to_be_bytes()); // array length
    for _ in 0..count {
        buf.extend_from_slice(&0_i64.to_be_bytes());
    }
}

// ── Light data ──────────────────────────────────────────────────────────

/// Encodes light data for an all-air chunk (1.18+).
///
/// For an empty chunk we mark all sections as having empty light (no arrays):
/// - `sky_light_mask`: empty BitSet
/// - `block_light_mask`: empty BitSet
/// - `empty_sky_light_mask`: BitSet with all section bits set
/// - `empty_block_light_mask`: BitSet with all section bits set
/// - `sky_light_array_count`: 0
/// - `block_light_array_count`: 0
fn encode_light_data(buf: &mut Vec<u8>, num_sections: usize, _version: ProtocolVersion) {
    // Number of bits needed: num_sections + 2 (for the two extra edge sections)
    let total_bits = num_sections + 2;
    // Number of longs needed to hold that many bits
    let num_longs: usize = (total_bits + 63) / 64;

    // Build the "all set" bitmask — one long with the lower `total_bits` bits set
    let all_set: u64 = if total_bits >= 64 {
        u64::MAX
    } else {
        (1_u64 << total_bits) - 1
    };

    // sky_light_mask: empty (no sky light arrays provided)
    write_varint(buf, num_longs as i32);
    for _ in 0..num_longs {
        buf.extend_from_slice(&0_u64.to_be_bytes());
    }

    // block_light_mask: empty
    write_varint(buf, num_longs as i32);
    for _ in 0..num_longs {
        buf.extend_from_slice(&0_u64.to_be_bytes());
    }

    // empty_sky_light_mask: all sections marked empty
    write_varint(buf, num_longs as i32);
    buf.extend_from_slice(&all_set.to_be_bytes());
    for _ in 1..num_longs {
        buf.extend_from_slice(&0_u64.to_be_bytes());
    }

    // empty_block_light_mask: all sections marked empty
    write_varint(buf, num_longs as i32);
    buf.extend_from_slice(&all_set.to_be_bytes());
    for _ in 1..num_longs {
        buf.extend_from_slice(&0_u64.to_be_bytes());
    }

    // sky_light_arrays: count = 0
    write_varint(buf, 0);

    // block_light_arrays: count = 0
    write_varint(buf, 0);
}

// ── VarInt helper ───────────────────────────────────────────────────────

/// Encodes a VarInt directly into a `Vec<u8>`.
///
/// This is a local helper to avoid pulling in the full codec trait machinery
/// just for raw byte construction. Also used by `spawn.rs` for raw packets.
pub(super) fn write_varint(buf: &mut Vec<u8>, value: i32) {
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

// ── Tests ───────────────────────────────────────────────────────────────

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
    fn test_chunk_data_payload_starts_correctly() {
        let payload = build_chunk_data_payload(3, -7, 16, ProtocolVersion::V1_21);
        assert_eq!(&payload[0..4], &3_i32.to_be_bytes());
        assert_eq!(&payload[4..8], &(-7_i32).to_be_bytes());
    }
}
