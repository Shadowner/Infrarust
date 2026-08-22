use crate::codec::McBufReadExt;
use crate::error::{ProtocolError, ProtocolResult};

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

const MAX_DEPTH: u32 = 512;

pub fn skip_nbt_compound(r: &mut &[u8]) -> ProtocolResult<()> {
    let tag_type = r.read_u8()?;
    if tag_type != TAG_COMPOUND {
        return Err(ProtocolError::invalid(format!(
            "expected NBT Compound (0x0A), got 0x{tag_type:02X}"
        )));
    }

    skip_nbt_string(r)?;

    skip_compound_payload(r, 0)
}

fn skip_compound_payload(r: &mut &[u8], depth: u32) -> ProtocolResult<()> {
    if depth > MAX_DEPTH {
        return Err(ProtocolError::invalid("NBT nesting depth exceeded"));
    }

    loop {
        let child_type = r.read_u8()?;
        if child_type == TAG_END {
            return Ok(());
        }

        skip_nbt_string(r)?;

        skip_tag_payload(r, child_type, depth)?;
    }
}

fn skip_tag_payload(r: &mut &[u8], tag_type: u8, depth: u32) -> ProtocolResult<()> {
    if depth > MAX_DEPTH {
        return Err(ProtocolError::invalid("NBT nesting depth exceeded"));
    }

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

fn skip_nbt_string(r: &mut &[u8]) -> ProtocolResult<()> {
    let len = r.read_u16_be()? as usize;
    skip_bytes(r, len)
}

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

    fn build_named_compound(name: &str, children: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(TAG_COMPOUND);
        buf.extend_from_slice(&(name.len() as u16).to_be_bytes());
        buf.extend_from_slice(name.as_bytes());
        buf.extend_from_slice(children);
        buf.push(TAG_END);
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
        let mut children = Vec::new();

        children.push(TAG_BYTE);
        children.extend_from_slice(&1u16.to_be_bytes());
        children.push(b'a');
        children.push(42);

        children.push(TAG_SHORT);
        children.extend_from_slice(&1u16.to_be_bytes());
        children.push(b'b');
        children.extend_from_slice(&1000i16.to_be_bytes());

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

        inner.push(TAG_COMPOUND);
        inner.extend_from_slice(&5u16.to_be_bytes());
        inner.extend_from_slice(b"inner");
        inner.push(TAG_LONG);
        inner.extend_from_slice(&3u16.to_be_bytes());
        inner.extend_from_slice(b"val");
        inner.extend_from_slice(&12345i64.to_be_bytes());
        inner.push(TAG_END);

        let data = build_named_compound("root", &inner);
        let mut r: &[u8] = &data;
        skip_nbt_compound(&mut r).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn test_skip_compound_with_list() {
        let mut children = Vec::new();

        children.push(TAG_LIST);
        children.extend_from_slice(&4u16.to_be_bytes());
        children.extend_from_slice(b"nums");
        children.push(TAG_INT);
        children.extend_from_slice(&3i32.to_be_bytes());
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
        let mut children = Vec::new();

        for i in 0..50 {
            let name = format!("entry_{i:03}");

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

        children.push(TAG_BYTE_ARRAY);
        children.extend_from_slice(&5u16.to_be_bytes());
        children.extend_from_slice(b"bytes");
        children.extend_from_slice(&5i32.to_be_bytes());
        children.extend_from_slice(&[1, 2, 3, 4, 5]);

        children.push(TAG_INT_ARRAY);
        children.extend_from_slice(&4u16.to_be_bytes());
        children.extend_from_slice(b"ints");
        children.extend_from_slice(&2i32.to_be_bytes());
        children.extend_from_slice(&10i32.to_be_bytes());
        children.extend_from_slice(&20i32.to_be_bytes());

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
        let data = [TAG_BYTE];
        let mut r: &[u8] = &data;
        assert!(skip_nbt_compound(&mut r).is_err());
    }

    fn build_nested_list_payload(levels: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.push(TAG_BYTE);
        payload.extend_from_slice(&0i32.to_be_bytes());
        for _ in 0..levels {
            let mut outer = Vec::new();
            outer.push(TAG_LIST);
            outer.extend_from_slice(&1i32.to_be_bytes());
            outer.extend_from_slice(&payload);
            payload = outer;
        }
        payload
    }

    #[test]
    fn test_deeply_nested_list_errors_instead_of_overflowing() {
        let mut children = Vec::new();
        children.push(TAG_LIST);
        children.extend_from_slice(&1u16.to_be_bytes());
        children.push(b'x');
        children.extend_from_slice(&build_nested_list_payload(MAX_DEPTH + 100));

        let data = build_named_compound("root", &children);
        let mut r: &[u8] = &data;
        let result = skip_nbt_compound(&mut r);
        assert!(
            result.is_err(),
            "deeply nested NBT lists must return Err, not overflow the stack"
        );
    }

    #[test]
    fn test_list_nesting_within_limit_ok() {
        let mut children = Vec::new();
        children.push(TAG_LIST);
        children.extend_from_slice(&1u16.to_be_bytes());
        children.push(b'x');
        children.extend_from_slice(&build_nested_list_payload(8));

        let data = build_named_compound("root", &children);
        let mut r: &[u8] = &data;
        skip_nbt_compound(&mut r).unwrap();
        assert!(r.is_empty());
    }
}
