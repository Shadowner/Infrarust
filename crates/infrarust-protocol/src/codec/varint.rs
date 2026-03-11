//! VarInt encoding and decoding for the Minecraft protocol.

use std::fmt;
use std::io::Write;

use crate::codec::{Decode, Encode};
use crate::error::{ProtocolError, ProtocolResult};

/// VarInt Minecraft — a signed 32-bit integer encoded in 1–5 bytes.
///
/// Each byte uses 7 bits for the value and 1 bit (MSB) as a continuation flag.
/// Used throughout the protocol: packet IDs, string lengths, array lengths, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VarInt(pub i32);

/// Result of a partial VarInt decode attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarIntDecodeStatus {
    /// Not enough bytes to complete the VarInt.
    Incomplete,
    /// VarInt exceeds maximum size (5 bytes for VarInt, 10 for VarLong).
    TooLarge,
}

impl VarInt {
    /// Maximum number of bytes a VarInt can occupy on the wire.
    pub const MAX_SIZE: usize = 5;

    /// Returns the number of bytes this VarInt will occupy when encoded.
    ///
    /// Computed in O(1) without loops.
    pub const fn written_size(self) -> usize {
        match self.0 {
            0 => 1,
            n => (31 - n.leading_zeros() as usize) / 7 + 1,
        }
    }

    /// Encodes this VarInt using a branchless algorithm (adapted from Valence).
    ///
    /// The algorithm spreads 7-bit groups into a u64 with gaps, computes the
    /// byte count via `leading_zeros`, inserts continuation bits in one OR, then
    /// writes the necessary bytes in little-endian order.
    pub fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
        let x = self.0 as u64;
        let stage1 = (x & 0x7f)
            | ((x & 0x3f80) << 1)
            | ((x & 0x1fc000) << 2)
            | ((x & 0xfe00000) << 3)
            | ((x & 0xf0000000) << 4);

        let leading = stage1.leading_zeros();
        let unused_bytes = (leading - 1) >> 3;
        let bytes_needed = (8 - unused_bytes) as usize;

        let msbs = 0x8080808080808080_u64;
        let msbmask = 0xffffffffffffffff_u64 >> (((8 - bytes_needed + 1) << 3) - 1);
        let merged = stage1 | (msbs & msbmask);

        let bytes = merged.to_le_bytes();
        debug_assert!((1..=5).contains(&bytes_needed));
        w.write_all(&bytes[..bytes_needed])?;
        Ok(())
    }

    /// Decodes a VarInt from a byte slice, advancing the cursor.
    ///
    /// Returns `ProtocolError::Incomplete` if the buffer is too short,
    /// or `ProtocolError::Invalid` if the VarInt exceeds 5 bytes.
    pub fn decode(r: &mut &[u8]) -> ProtocolResult<Self> {
        let mut val = 0i32;
        for i in 0..Self::MAX_SIZE {
            if r.is_empty() {
                return Err(ProtocolError::Incomplete { context: "VarInt" });
            }
            let byte = r[0];
            *r = &r[1..];
            val |= (i32::from(byte) & 0x7F) << (i * 7);
            if byte & 0x80 == 0 {
                return Ok(VarInt(val));
            }
        }
        Err(ProtocolError::invalid("VarInt too large (> 5 bytes)"))
    }

    /// Attempts to decode a VarInt without consuming the buffer.
    ///
    /// Used by the packet decoder for framing — we need to try reading the
    /// length VarInt without advancing the cursor if it's incomplete.
    ///
    /// Returns `(value, bytes_consumed)` on success.
    pub fn decode_partial(buf: &[u8]) -> Result<(VarInt, usize), VarIntDecodeStatus> {
        let mut val = 0i32;
        for i in 0..Self::MAX_SIZE {
            if i >= buf.len() {
                return Err(VarIntDecodeStatus::Incomplete);
            }
            let byte = buf[i];
            val |= (i32::from(byte) & 0x7F) << (i * 7);
            if byte & 0x80 == 0 {
                return Ok((VarInt(val), i + 1));
            }
        }
        Err(VarIntDecodeStatus::TooLarge)
    }
}

impl Encode for VarInt {
    fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
        VarInt::encode(self, w)
    }
}

impl Decode<'_> for VarInt {
    fn decode(r: &mut &[u8]) -> ProtocolResult<Self> {
        VarInt::decode(r)
    }
}

impl From<i32> for VarInt {
    fn from(val: i32) -> Self {
        VarInt(val)
    }
}

impl From<VarInt> for i32 {
    fn from(val: VarInt) -> Self {
        val.0
    }
}

impl fmt::Display for VarInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUND_TRIP_VALUES: &[i32] = &[
        0,
        1,
        -1,
        2,
        127,
        128,
        255,
        25565,
        2097151,
        i32::MIN,
        i32::MAX,
    ];

    #[test]
    fn test_encode_decode_round_trip() {
        for &val in ROUND_TRIP_VALUES {
            let vi = VarInt(val);
            let mut buf = Vec::new();
            vi.encode(&mut buf).unwrap();
            let mut slice: &[u8] = &buf;
            let decoded = VarInt::decode(&mut slice).unwrap();
            assert_eq!(vi, decoded, "round-trip failed for {val}");
            assert!(slice.is_empty(), "trailing bytes for {val}");
        }
    }

    #[test]
    fn test_written_size_matches_actual_encoded_length() {
        for &val in ROUND_TRIP_VALUES {
            let vi = VarInt(val);
            let mut buf = Vec::new();
            vi.encode(&mut buf).unwrap();
            assert_eq!(
                vi.written_size(),
                buf.len(),
                "written_size mismatch for {val}"
            );
        }
    }

    #[test]
    fn test_decode_incomplete_buffer() {
        let mut buf: &[u8] = &[0x80, 0x80];
        let err = VarInt::decode(&mut buf).unwrap_err();
        assert!(err.is_incomplete());
    }

    #[test]
    fn test_decode_too_large() {
        let mut buf: &[u8] = &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80];
        let err = VarInt::decode(&mut buf).unwrap_err();
        assert!(err.is_fatal());
    }

    #[test]
    fn test_decode_partial_success() {
        let vi = VarInt(300);
        let mut encoded = Vec::new();
        vi.encode(&mut encoded).unwrap();
        encoded.extend_from_slice(&[0xDE, 0xAD]);

        let (decoded, consumed) = VarInt::decode_partial(&encoded).unwrap();
        assert_eq!(decoded, vi);
        assert_eq!(consumed, 2);
    }

    #[test]
    fn test_decode_partial_incomplete() {
        let result = VarInt::decode_partial(&[0x80, 0x80]);
        assert_eq!(result, Err(VarIntDecodeStatus::Incomplete));
    }

    #[test]
    fn test_decode_partial_too_large() {
        let result = VarInt::decode_partial(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80]);
        assert_eq!(result, Err(VarIntDecodeStatus::TooLarge));
    }

    #[test]
    fn test_zero_is_one_byte() {
        let mut buf = Vec::new();
        VarInt(0).encode(&mut buf).unwrap();
        assert_eq!(buf, [0x00]);
    }

    #[test]
    fn test_negative_one_is_five_bytes() {
        let mut buf = Vec::new();
        VarInt(-1).encode(&mut buf).unwrap();
        assert_eq!(buf, [0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
    }

    #[test]
    fn test_from_i32_conversion() {
        assert_eq!(VarInt::from(42).0, 42);
        assert_eq!(i32::from(VarInt(42)), 42);
    }
}
