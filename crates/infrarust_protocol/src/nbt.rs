//! Minimal NBT skipper for network protocol parsing.
//!
//! Skips NBT Compound tags without interpreting their contents. This is used
//! to parse JoinGame packets in 1.16-1.16.1 where `dimension_codec` and
//! `dimension_type` are large NBT Compounds that must be skipped to reach
//! the `dimension_name` field.

use crate::codec::McBufReadExt;
use crate::error::{ProtocolError, ProtocolResult};

// NBT tag type IDs
const TAG_END: u8 = 0;
const TAG_BYTE: u8 = 1;
const TAG_SHORT: u8 = 2;
const TAG_INT: u8 = 3;
const TAG_LONG: u8 = 4;
const TAG_FLOAT: u8 = 5;
const TAG_DOUBLE: u8 = 6;
const TAG_BYTE_ARRAY: u8 = 7;
const TAG_STRING: u8 = 8;
const TAG_LIST: u8 = 9;
const TAG_COMPOUND: u8 = 10;
const TAG_INT_ARRAY: u8 = 11;
const TAG_LONG_ARRAY: u8 = 12;

/// Maximum nesting depth to prevent stack overflow from malicious input.
const MAX_DEPTH: u32 = 512;

/// Skips a full NBT Compound tag including the root tag type byte and name.
///
/// Network NBT format (pre-1.20.2):
/// - Tag type byte (must be 0x0A = Compound)
/// - Root tag name: u16 BE length + UTF-8 bytes
/// - Compound payload (children terminated by TAG_End)
///
/// # Errors
/// Returns `ProtocolError` if the data is truncated, malformed, or exceeds depth limits.
pub fn skip_nbt_compound(r: &mut &[u8]) -> ProtocolResult<()> {
    let tag_type = r.read_u8()?;
    if tag_type != TAG_COMPOUND {
        return Err(ProtocolError::invalid(format!(
            "expected NBT Compound (0x0A), got 0x{tag_type:02X}"
        )));
    }

    // Skip root tag name (u16 BE length + bytes)
    skip_nbt_string(r)?;

    // Skip compound payload
    skip_compound_payload(r, 0)
}

/// Skips a full NBT Compound tag in 1.20.2+ network format (no root name).
///
/// 1.20.2+ network NBT:
/// - Tag type byte (must be 0x0A = Compound)
/// - Compound payload (children terminated by TAG_End) — NO root name
pub fn _skip_nbt_compound_nameless(r: &mut &[u8]) -> ProtocolResult<()> {
    let tag_type = r.read_u8()?;
    if tag_type != TAG_COMPOUND {
        return Err(ProtocolError::invalid(format!(
            "expected NBT Compound (0x0A), got 0x{tag_type:02X}"
        )));
    }
    skip_compound_payload(r, 0)
}

/// Skips the payload of a compound tag (children until TAG_End).
fn skip_compound_payload(r: &mut &[u8], depth: u32) -> ProtocolResult<()> {
    if depth > MAX_DEPTH {
        return Err(ProtocolError::invalid("NBT nesting depth exceeded"));
    }

    loop {
        let child_type = r.read_u8()?;
        if child_type == TAG_END {
            return Ok(());
        }

        // Skip child tag name
        skip_nbt_string(r)?;

        // Skip child tag payload
        skip_tag_payload(r, child_type, depth)?;
    }
}

/// Skips the payload of a single tag (not including type byte or name).
fn skip_tag_payload(r: &mut &[u8], tag_type: u8, depth: u32) -> ProtocolResult<()> {
    match tag_type {
        TAG_BYTE => skip_bytes(r, 1),
        TAG_SHORT => skip_bytes(r, 2),
        TAG_INT | TAG_FLOAT => skip_bytes(r, 4),
        TAG_LONG | TAG_DOUBLE => skip_bytes(r, 8),
        TAG_BYTE_ARRAY => {
            let raw_len = r.read_i32_be()?;
            if raw_len < 0 {
                return Err(ProtocolError::invalid("negative NBT byte array length"));
            }
            skip_bytes(r, raw_len as usize)
        }
        TAG_STRING => skip_nbt_string(r),
        TAG_LIST => {
            let element_type = r.read_u8()?;
            let count = r.read_i32_be()?;
            if count <= 0 {
                return Ok(());
            }
            for _ in 0..count {
                skip_tag_payload(r, element_type, depth + 1)?;
            }
            Ok(())
        }
        TAG_COMPOUND => skip_compound_payload(r, depth + 1),
        TAG_INT_ARRAY => {
            let raw_count = r.read_i32_be()?;
            if raw_count < 0 {
                return Err(ProtocolError::invalid("negative NBT int array count"));
            }
            let byte_len = (raw_count as usize)
                .checked_mul(4)
                .ok_or_else(|| ProtocolError::invalid("NBT int array size overflow"))?;
            skip_bytes(r, byte_len)
        }
        TAG_LONG_ARRAY => {
            let raw_count = r.read_i32_be()?;
            if raw_count < 0 {
                return Err(ProtocolError::invalid("negative NBT long array count"));
            }
            let byte_len = (raw_count as usize)
                .checked_mul(8)
                .ok_or_else(|| ProtocolError::invalid("NBT long array size overflow"))?;
            skip_bytes(r, byte_len)
        }
        _ => Err(ProtocolError::invalid(format!(
            "unknown NBT tag type: {tag_type}"
        ))),
    }
}

/// Skips an NBT string (u16 BE length + UTF-8 bytes).
fn skip_nbt_string(r: &mut &[u8]) -> ProtocolResult<()> {
    let len = r.read_u16_be()? as usize;
    skip_bytes(r, len)
}

/// Advances the reader by `n` bytes.
fn skip_bytes(r: &mut &[u8], n: usize) -> ProtocolResult<()> {
    if r.len() < n {
        return Err(ProtocolError::Incomplete {
            context: "NBT skip",
        });
    }
    *r = &r[n..];
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// Builds an NBT compound with a named root.
    fn build_named_compound(name: &str, children: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(TAG_COMPOUND); // root type
        buf.extend_from_slice(&(name.len() as u16).to_be_bytes()); // root name len
        buf.extend_from_slice(name.as_bytes()); // root name
        buf.extend_from_slice(children); // compound payload
        buf.push(TAG_END); // end of compound
        buf
    }

    #[test]
    fn test_skip_empty_compound() {
        let data = build_named_compound("", &[]);
        let mut r: &[u8] = &data;
        skip_nbt_compound(&mut r).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn test_skip_simple_compound() {
        // Compound with: byte "a"=42, short "b"=1000, int "c"=100000
        let mut children = Vec::new();

        // TAG_Byte "a" = 42
        children.push(TAG_BYTE);
        children.extend_from_slice(&1u16.to_be_bytes());
        children.push(b'a');
        children.push(42);

        // TAG_Short "b" = 1000
        children.push(TAG_SHORT);
        children.extend_from_slice(&1u16.to_be_bytes());
        children.push(b'b');
        children.extend_from_slice(&1000i16.to_be_bytes());

        // TAG_Int "c" = 100000
        children.push(TAG_INT);
        children.extend_from_slice(&1u16.to_be_bytes());
        children.push(b'c');
        children.extend_from_slice(&100_000i32.to_be_bytes());

        let data = build_named_compound("root", &children);
        let mut r: &[u8] = &data;
        skip_nbt_compound(&mut r).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn test_skip_nested_compound() {
        let mut inner = Vec::new();

        // Inner compound "inner" with TAG_Long "val" = 12345
        inner.push(TAG_COMPOUND);
        inner.extend_from_slice(&5u16.to_be_bytes());
        inner.extend_from_slice(b"inner");
        // child: TAG_Long "val" = 12345
        inner.push(TAG_LONG);
        inner.extend_from_slice(&3u16.to_be_bytes());
        inner.extend_from_slice(b"val");
        inner.extend_from_slice(&12345i64.to_be_bytes());
        inner.push(TAG_END); // end inner compound

        let data = build_named_compound("root", &inner);
        let mut r: &[u8] = &data;
        skip_nbt_compound(&mut r).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn test_skip_compound_with_list() {
        let mut children = Vec::new();

        // TAG_List "nums" of TAG_Int, count=3
        children.push(TAG_LIST);
        children.extend_from_slice(&4u16.to_be_bytes());
        children.extend_from_slice(b"nums");
        children.push(TAG_INT); // element type
        children.extend_from_slice(&3i32.to_be_bytes()); // count
        children.extend_from_slice(&1i32.to_be_bytes());
        children.extend_from_slice(&2i32.to_be_bytes());
        children.extend_from_slice(&3i32.to_be_bytes());

        let data = build_named_compound("root", &children);
        let mut r: &[u8] = &data;
        skip_nbt_compound(&mut r).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn test_skip_large_compound() {
        // Simulate a ~2KB dimension_codec with many string and byte array entries
        let mut children = Vec::new();

        for i in 0..50 {
            let name = format!("entry_{i:03}");

            // TAG_String "entry_NNN" = "minecraft:some_long_dimension_type_name_..."
            children.push(TAG_STRING);
            children.extend_from_slice(&(name.len() as u16).to_be_bytes());
            children.extend_from_slice(name.as_bytes());
            let value = format!("minecraft:dimension_type_{i}_with_padding_data_here");
            children.extend_from_slice(&(value.len() as u16).to_be_bytes());
            children.extend_from_slice(value.as_bytes());
        }

        let data = build_named_compound("dimension_codec", &children);
        assert!(
            data.len() > 2000,
            "compound should be ~2KB, got {} bytes",
            data.len()
        );

        let mut r: &[u8] = &data;
        skip_nbt_compound(&mut r).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn test_skip_compound_with_byte_array_and_int_array() {
        let mut children = Vec::new();

        // TAG_ByteArray "bytes" = [1, 2, 3, 4, 5]
        children.push(TAG_BYTE_ARRAY);
        children.extend_from_slice(&5u16.to_be_bytes());
        children.extend_from_slice(b"bytes");
        children.extend_from_slice(&5i32.to_be_bytes());
        children.extend_from_slice(&[1, 2, 3, 4, 5]);

        // TAG_IntArray "ints" = [10, 20]
        children.push(TAG_INT_ARRAY);
        children.extend_from_slice(&4u16.to_be_bytes());
        children.extend_from_slice(b"ints");
        children.extend_from_slice(&2i32.to_be_bytes());
        children.extend_from_slice(&10i32.to_be_bytes());
        children.extend_from_slice(&20i32.to_be_bytes());

        // TAG_LongArray "longs" = [100]
        children.push(TAG_LONG_ARRAY);
        children.extend_from_slice(&5u16.to_be_bytes());
        children.extend_from_slice(b"longs");
        children.extend_from_slice(&1i32.to_be_bytes());
        children.extend_from_slice(&100i64.to_be_bytes());

        let data = build_named_compound("root", &children);
        let mut r: &[u8] = &data;
        skip_nbt_compound(&mut r).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn test_data_after_compound_preserved() {
        let data = build_named_compound("", &[]);
        let mut full = data.clone();
        full.extend_from_slice(&[0xDE, 0xAD]);

        let mut r: &[u8] = &full;
        skip_nbt_compound(&mut r).unwrap();
        assert_eq!(r, &[0xDE, 0xAD]);
    }

    #[test]
    fn test_wrong_tag_type_errors() {
        let data = [TAG_BYTE]; // Not a compound
        let mut r: &[u8] = &data;
        assert!(skip_nbt_compound(&mut r).is_err());
    }
}
