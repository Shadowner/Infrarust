//! Encode/Decode implementations for Minecraft primitive types.
//!
//! All numeric types use big-endian byte order (Minecraft standard).
//! Strings are VarInt-prefixed UTF-8. UUIDs are encoded as u128 big-endian.

use std::io::{Read, Write};

use uuid::Uuid;

use crate::codec::varint::VarInt;
use crate::codec::{Decode, Encode};
use crate::error::{ProtocolError, ProtocolResult};

/// Default maximum string length in characters (Minecraft protocol limit).
const MAX_STRING_CHARS: usize = 32767;

/// Computes a cautious pre-allocation capacity.
///
/// Prevents a malicious VarInt length prefix from causing a massive allocation
/// before any data has been read from the network.
fn cautious_capacity(size_hint: usize) -> usize {
    const MAX_PREALLOC_BYTES: usize = 1024 * 1024; // 1 MB
    size_hint.min(MAX_PREALLOC_BYTES)
}

// --- bool ---

impl Encode for bool {
    fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
        w.write_all(&[u8::from(*self)])?;
        Ok(())
    }
}

impl Decode<'_> for bool {
    fn decode(r: &mut &[u8]) -> ProtocolResult<Self> {
        let byte = u8::decode(r)?;
        match byte {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ProtocolError::invalid("bool value must be 0 or 1")),
        }
    }
}

// --- u8 ---

impl Encode for u8 {
    fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
        w.write_all(&[*self])?;
        Ok(())
    }
}

impl Decode<'_> for u8 {
    fn decode(r: &mut &[u8]) -> ProtocolResult<Self> {
        if r.is_empty() {
            return Err(ProtocolError::Incomplete { context: "u8" });
        }
        let val = r[0];
        *r = &r[1..];
        Ok(val)
    }
}

// --- i8 ---

impl Encode for i8 {
    fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
        w.write_all(&[*self as u8])?;
        Ok(())
    }
}

impl Decode<'_> for i8 {
    fn decode(r: &mut &[u8]) -> ProtocolResult<Self> {
        let byte = u8::decode(r)?;
        Ok(byte as i8)
    }
}

// --- Macro for fixed-size big-endian numeric types ---

macro_rules! impl_codec_be {
    ($ty:ty, $size:literal, $ctx:literal) => {
        impl Encode for $ty {
            fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
                w.write_all(&self.to_be_bytes())?;
                Ok(())
            }
        }

        impl Decode<'_> for $ty {
            fn decode(r: &mut &[u8]) -> ProtocolResult<Self> {
                if r.len() < $size {
                    return Err(ProtocolError::Incomplete { context: $ctx });
                }
                let (bytes, rest) = r.split_at($size);
                let val =
                    <$ty>::from_be_bytes(bytes.try_into().expect("split_at guarantees length"));
                *r = rest;
                Ok(val)
            }
        }
    };
}

impl_codec_be!(u16, 2, "u16");
impl_codec_be!(i16, 2, "i16");
impl_codec_be!(u32, 4, "u32");
impl_codec_be!(i32, 4, "i32");
impl_codec_be!(u64, 8, "u64");
impl_codec_be!(i64, 8, "i64");
impl_codec_be!(u128, 16, "u128");
impl_codec_be!(f32, 4, "f32");
impl_codec_be!(f64, 8, "f64");

// --- String ---

impl Encode for String {
    fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
        encode_string(self.as_str(), w)
    }
}

impl Decode<'_> for String {
    fn decode(r: &mut &[u8]) -> ProtocolResult<Self> {
        decode_string(r, MAX_STRING_CHARS)
    }
}

/// Encodes a string with VarInt length prefix.
pub(crate) fn encode_string(s: &str, w: &mut impl Write) -> ProtocolResult<()> {
    let char_len = s.chars().count();
    if char_len > MAX_STRING_CHARS {
        return Err(ProtocolError::too_large(MAX_STRING_CHARS, char_len));
    }
    let bytes = s.as_bytes();
    VarInt(bytes.len() as i32).encode(w)?;
    w.write_all(bytes)?;
    Ok(())
}

/// Decodes a VarInt-prefixed UTF-8 string with a character limit.
pub(crate) fn decode_string(r: &mut &[u8], max_chars: usize) -> ProtocolResult<String> {
    let raw_len = VarInt::decode(r)?.0;
    if raw_len < 0 {
        return Err(ProtocolError::invalid("negative length"));
    }
    let byte_len = raw_len as usize;
    // A single UTF-8 char is at most 4 bytes
    let max_bytes = max_chars * 4;
    if byte_len > max_bytes {
        return Err(ProtocolError::too_large(max_bytes, byte_len));
    }
    if r.len() < byte_len {
        return Err(ProtocolError::Incomplete {
            context: "String data",
        });
    }
    let (data, rest) = r.split_at(byte_len);
    let s = String::from_utf8(data.to_vec())
        .map_err(|_| ProtocolError::invalid("invalid UTF-8 in string"))?;
    if s.chars().count() > max_chars {
        return Err(ProtocolError::too_large(max_chars, s.chars().count()));
    }
    *r = rest;
    Ok(s)
}

// --- uuid::Uuid ---

impl Encode for Uuid {
    fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
        self.as_u128().encode(w)
    }
}

impl Decode<'_> for Uuid {
    fn decode(r: &mut &[u8]) -> ProtocolResult<Self> {
        let val = u128::decode(r)?;
        Ok(Uuid::from_u128(val))
    }
}

// --- Vec<u8> (byte array with VarInt length prefix) ---

impl Encode for Vec<u8> {
    fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
        VarInt(self.len() as i32).encode(w)?;
        w.write_all(self)?;
        Ok(())
    }
}

impl Decode<'_> for Vec<u8> {
    fn decode(r: &mut &[u8]) -> ProtocolResult<Self> {
        let raw_len = VarInt::decode(r)?.0;
        if raw_len < 0 {
            return Err(ProtocolError::invalid("negative length"));
        }
        let len = raw_len as usize;
        if r.len() < len {
            return Err(ProtocolError::Incomplete {
                context: "byte array",
            });
        }
        let mut buf = Vec::with_capacity(cautious_capacity(len));
        let (data, rest) = r.split_at(len);
        buf.extend_from_slice(data);
        *r = rest;
        Ok(buf)
    }
}

// --- Option<T> ---

impl<T: Encode> Encode for Option<T> {
    fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
        match self {
            Some(val) => {
                true.encode(w)?;
                val.encode(w)?;
            }
            None => {
                false.encode(w)?;
            }
        }
        Ok(())
    }
}

impl<'a, T: Decode<'a>> Decode<'a> for Option<T> {
    fn decode(r: &mut &'a [u8]) -> ProtocolResult<Self> {
        let present = bool::decode(r)?;
        if present {
            Ok(Some(T::decode(r)?))
        } else {
            Ok(None)
        }
    }
}

// --- McBufReadExt / McBufWriteExt helper functions used by codec/mod.rs ---

/// Reads a bounded string from a `Read` source.
pub(crate) fn read_string_bounded_from_reader(
    reader: &mut impl Read,
    max_chars: usize,
) -> ProtocolResult<String> {
    let raw_len = read_varint_from_reader(reader)?.0;
    if raw_len < 0 {
        return Err(ProtocolError::invalid("negative length"));
    }
    let byte_len = raw_len as usize;
    let max_bytes = max_chars * 4;
    if byte_len > max_bytes {
        return Err(ProtocolError::too_large(max_bytes, byte_len));
    }
    let mut buf = vec![0u8; byte_len];
    reader.read_exact(&mut buf)?;
    let s =
        String::from_utf8(buf).map_err(|_| ProtocolError::invalid("invalid UTF-8 in string"))?;
    if s.chars().count() > max_chars {
        return Err(ProtocolError::too_large(max_chars, s.chars().count()));
    }
    Ok(s)
}

/// Reads a VarInt from a `Read` source (byte-by-byte).
pub(crate) fn read_varint_from_reader(reader: &mut impl Read) -> ProtocolResult<VarInt> {
    let mut val = 0i32;
    for i in 0..VarInt::MAX_SIZE {
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte)?;
        val |= (i32::from(byte[0]) & 0x7F) << (i * 7);
        if byte[0] & 0x80 == 0 {
            return Ok(VarInt(val));
        }
    }
    Err(ProtocolError::invalid("VarInt too large (> 5 bytes)"))
}

/// Reads a VarLong from a `Read` source (byte-by-byte).
pub(crate) fn read_varlong_from_reader(
    reader: &mut impl Read,
) -> ProtocolResult<crate::codec::varlong::VarLong> {
    use crate::codec::varlong::VarLong;
    let mut val = 0i64;
    for i in 0..VarLong::MAX_SIZE {
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte)?;
        val |= (i64::from(byte[0]) & 0x7F) << (i * 7);
        if byte[0] & 0x80 == 0 {
            return Ok(VarLong(val));
        }
    }
    Err(ProtocolError::invalid("VarLong too large (> 10 bytes)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- bool ---

    #[test]
    fn test_bool_round_trip() {
        for &val in &[true, false] {
            let mut buf = Vec::new();
            val.encode(&mut buf).unwrap();
            let mut slice: &[u8] = &buf;
            let decoded = bool::decode(&mut slice).unwrap();
            assert_eq!(val, decoded);
        }
    }

    #[test]
    fn test_bool_invalid_value() {
        let mut slice: &[u8] = &[0x02];
        let err = bool::decode(&mut slice).unwrap_err();
        assert!(err.is_fatal());
    }

    // --- numerics ---

    #[test]
    fn test_u8_round_trip() {
        for &val in &[0u8, 128, 255] {
            let mut buf = Vec::new();
            val.encode(&mut buf).unwrap();
            let mut slice: &[u8] = &buf;
            assert_eq!(u8::decode(&mut slice).unwrap(), val);
        }
    }

    #[test]
    fn test_i32_round_trip_big_endian() {
        let mut buf = Vec::new();
        1i32.encode(&mut buf).unwrap();
        assert_eq!(buf, [0, 0, 0, 1], "i32 should be big-endian");
        let mut slice: &[u8] = &buf;
        assert_eq!(i32::decode(&mut slice).unwrap(), 1);
    }

    #[test]
    fn test_f32_round_trip() {
        for &val in &[0.0f32, 1.0, -1.0, 3.14] {
            let mut buf = Vec::new();
            val.encode(&mut buf).unwrap();
            let mut slice: &[u8] = &buf;
            let decoded = f32::decode(&mut slice).unwrap();
            assert!(
                (val - decoded).abs() < f32::EPSILON,
                "f32 round-trip for {val}"
            );
        }
    }

    #[test]
    fn test_f64_round_trip() {
        for &val in &[0.0f64, 1.0, -1.0, 3.14] {
            let mut buf = Vec::new();
            val.encode(&mut buf).unwrap();
            let mut slice: &[u8] = &buf;
            let decoded = f64::decode(&mut slice).unwrap();
            assert!(
                (val - decoded).abs() < f64::EPSILON,
                "f64 round-trip for {val}"
            );
        }
    }

    #[test]
    fn test_i64_round_trip() {
        for &val in &[0i64, -1, i64::MAX, i64::MIN] {
            let mut buf = Vec::new();
            val.encode(&mut buf).unwrap();
            let mut slice: &[u8] = &buf;
            assert_eq!(i64::decode(&mut slice).unwrap(), val);
        }
    }

    // --- String ---

    #[test]
    fn test_string_round_trip() {
        for s in &["Hello", "", "Héllo 🌍"] {
            let val = s.to_string();
            let mut buf = Vec::new();
            val.encode(&mut buf).unwrap();
            let mut slice: &[u8] = &buf;
            assert_eq!(String::decode(&mut slice).unwrap(), *s);
        }
    }

    #[test]
    fn test_string_length_prefix_is_varint() {
        let val = "AB".to_string();
        let mut buf = Vec::new();
        val.encode(&mut buf).unwrap();
        // VarInt(2) = 0x02, then ASCII 'A' = 0x41, 'B' = 0x42
        assert_eq!(buf, [0x02, 0x41, 0x42]);
    }

    #[test]
    fn test_string_too_long() {
        let long_string: String = "x".repeat(40000);
        let mut buf = Vec::new();
        let err = long_string.encode(&mut buf).unwrap_err();
        assert!(matches!(err, ProtocolError::TooLarge { .. }));
    }

    #[test]
    fn test_string_bounded_read() {
        let val = "a]".repeat(20);
        let mut buf = Vec::new();
        val.encode(&mut buf).unwrap();
        let mut slice: &[u8] = &buf;
        let err = decode_string(&mut slice, 10).unwrap_err();
        assert!(err.is_fatal());
    }

    // --- UUID ---

    #[test]
    fn test_uuid_round_trip() {
        let uuid = Uuid::from_u128(0x550e8400_e29b_41d4_a716_446655440000);
        let mut buf = Vec::new();
        uuid.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), 16);
        let mut slice: &[u8] = &buf;
        assert_eq!(Uuid::decode(&mut slice).unwrap(), uuid);
    }

    #[test]
    fn test_uuid_is_big_endian() {
        let uuid = Uuid::from_u128(0xFF00000000000000_0000000000000000);
        let mut buf = Vec::new();
        uuid.encode(&mut buf).unwrap();
        assert_eq!(buf[0], 0xFF, "first byte should be MSB of u128");
    }

    // --- Vec<u8> ---

    #[test]
    fn test_byte_array_round_trip() {
        let data: Vec<u8> = vec![1, 2, 3, 4, 5];
        let mut buf = Vec::new();
        data.encode(&mut buf).unwrap();
        // VarInt(5) = 0x05
        assert_eq!(buf[0], 0x05);
        let mut slice: &[u8] = &buf;
        assert_eq!(Vec::<u8>::decode(&mut slice).unwrap(), data);
    }

    #[test]
    fn test_byte_array_empty() {
        let data: Vec<u8> = vec![];
        let mut buf = Vec::new();
        data.encode(&mut buf).unwrap();
        assert_eq!(buf, [0x00]); // VarInt(0) only
        let mut slice: &[u8] = &buf;
        assert_eq!(Vec::<u8>::decode(&mut slice).unwrap(), data);
    }

    // --- Option ---

    #[test]
    fn test_option_some_round_trip() {
        let val: Option<i32> = Some(42);
        let mut buf = Vec::new();
        val.encode(&mut buf).unwrap();
        assert_eq!(buf[0], 0x01); // true prefix
        let mut slice: &[u8] = &buf;
        assert_eq!(Option::<i32>::decode(&mut slice).unwrap(), Some(42));
    }

    #[test]
    fn test_option_none_round_trip() {
        let val: Option<i32> = None;
        let mut buf = Vec::new();
        val.encode(&mut buf).unwrap();
        assert_eq!(buf, [0x00]); // false prefix only
        let mut slice: &[u8] = &buf;
        assert_eq!(Option::<i32>::decode(&mut slice).unwrap(), None);
    }

    // --- Extension trait consistency ---

    #[test]
    fn test_mcbuf_read_write_consistency() {
        use crate::codec::{McBufReadExt, McBufWriteExt};
        use std::io::Cursor;

        let mut buf = Vec::new();
        buf.write_u8(42).unwrap();
        buf.write_i8(-1).unwrap();
        buf.write_u16_be(1234).unwrap();
        buf.write_i16_be(-567).unwrap();
        buf.write_u32_be(0xDEADBEEF).unwrap();
        buf.write_i32_be(-42).unwrap();
        buf.write_u64_be(0xCAFEBABE).unwrap();
        buf.write_i64_be(-99).unwrap();
        buf.write_f32_be(3.14).unwrap();
        buf.write_f64_be(2.718).unwrap();
        buf.write_bool(true).unwrap();
        buf.write_string("hello").unwrap();
        buf.write_uuid(&Uuid::from_u128(123456)).unwrap();

        let mut reader = Cursor::new(buf);
        assert_eq!(reader.read_u8().unwrap(), 42);
        assert_eq!(reader.read_i8().unwrap(), -1);
        assert_eq!(reader.read_u16_be().unwrap(), 1234);
        assert_eq!(reader.read_i16_be().unwrap(), -567);
        assert_eq!(reader.read_u32_be().unwrap(), 0xDEADBEEF);
        assert_eq!(reader.read_i32_be().unwrap(), -42);
        assert_eq!(reader.read_u64_be().unwrap(), 0xCAFEBABE);
        assert_eq!(reader.read_i64_be().unwrap(), -99);
        let f32_val = reader.read_f32_be().unwrap();
        assert!((f32_val - 3.14).abs() < f32::EPSILON);
        let f64_val = reader.read_f64_be().unwrap();
        assert!((f64_val - 2.718).abs() < f64::EPSILON);
        assert!(reader.read_bool().unwrap());
        assert_eq!(reader.read_string().unwrap(), "hello");
        assert_eq!(reader.read_uuid().unwrap(), Uuid::from_u128(123456));
    }

    #[test]
    fn test_mcbuf_var_int_via_extension() {
        use crate::codec::{McBufReadExt, McBufWriteExt};
        use std::io::Cursor;

        let mut buf = Vec::new();
        buf.write_var_int(&VarInt(300)).unwrap();
        buf.write_var_long(&crate::codec::varlong::VarLong(i64::MAX))
            .unwrap();

        let mut reader = Cursor::new(buf);
        assert_eq!(reader.read_var_int().unwrap(), VarInt(300));
        assert_eq!(
            reader.read_var_long().unwrap(),
            crate::codec::varlong::VarLong(i64::MAX)
        );
    }

    #[test]
    fn test_string_decode_invalid_utf8() {
        let data = {
            let mut buf = Vec::new();
            VarInt(2).encode(&mut buf).unwrap();
            buf.extend_from_slice(&[0xFF, 0xFE]); // invalid UTF-8
            buf
        };
        let mut cursor = data.as_slice();
        let result = String::decode(&mut cursor);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ProtocolError::Invalid { .. }));
    }

    #[test]
    fn test_string_decode_negative_length() {
        let mut buf = Vec::new();
        VarInt(-1).encode(&mut buf).unwrap();
        let mut cursor = buf.as_slice();
        let result = String::decode(&mut cursor);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ProtocolError::Invalid { .. }));
    }

    #[test]
    fn test_byte_array_decode_negative_length() {
        let mut buf = Vec::new();
        VarInt(-1).encode(&mut buf).unwrap();
        let mut cursor = buf.as_slice();
        let result = Vec::<u8>::decode(&mut cursor);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ProtocolError::Invalid { .. }));
    }

    #[test]
    fn test_read_byte_array_negative_length() {
        use crate::codec::McBufReadExt;
        use std::io::Cursor;

        let mut buf = Vec::new();
        VarInt(-5).encode(&mut buf).unwrap();
        let mut reader = Cursor::new(buf);
        let result = reader.read_byte_array(1024);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ProtocolError::Invalid { .. }));
    }

    #[test]
    fn test_read_string_negative_length() {
        use crate::codec::McBufReadExt;
        use std::io::Cursor;

        let mut buf = Vec::new();
        VarInt(-1).encode(&mut buf).unwrap();
        let mut reader = Cursor::new(buf);
        let result = reader.read_string();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ProtocolError::Invalid { .. }));
    }
}
