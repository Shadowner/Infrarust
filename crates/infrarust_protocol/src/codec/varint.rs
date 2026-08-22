use std::fmt;
use std::io::Write;

use crate::codec::{Decode, Encode};
use crate::error::{ProtocolError, ProtocolResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VarInt(pub i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarIntDecodeStatus {
    Incomplete,
    TooLarge,
}

impl VarInt {
    pub const MAX_SIZE: usize = 5;

    pub const fn written_size(self) -> usize {
        match self.0 {
            0 => 1,
            n => (31 - n.leading_zeros() as usize) / 7 + 1,
        }
    }

    pub fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
        let x = self.0 as u64;
        let stage1 = (x & 0x7f)
            | ((x & 0x3f80) << 1)
            | ((x & 0x001f_c000) << 2)
            | ((x & 0x0fe0_0000) << 3)
            | ((x & 0xf000_0000) << 4);

        let leading = stage1.leading_zeros();
        let unused_bytes = (leading - 1) >> 3;
        let bytes_needed = (8 - unused_bytes) as usize;

        let msbs = 0x8080_8080_8080_8080_u64;
        let msbmask = 0xffff_ffff_ffff_ffff_u64 >> (((8 - bytes_needed + 1) << 3) - 1);
        let merged = stage1 | (msbs & msbmask);

        let bytes = merged.to_le_bytes();
        debug_assert!((1..=5).contains(&bytes_needed));
        w.write_all(&bytes[..bytes_needed])?;
        Ok(())
    }

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
                return Ok(Self(val));
            }
        }
        Err(ProtocolError::invalid("VarInt too large (> 5 bytes)"))
    }

    pub fn decode_partial(buf: &[u8]) -> Result<(Self, usize), VarIntDecodeStatus> {
        let mut val = 0i32;
        for i in 0..Self::MAX_SIZE {
            if i >= buf.len() {
                return Err(VarIntDecodeStatus::Incomplete);
            }
            let byte = buf[i];
            val |= (i32::from(byte) & 0x7F) << (i * 7);
            if byte & 0x80 == 0 {
                return Ok((Self(val), i + 1));
            }
        }
        Err(VarIntDecodeStatus::TooLarge)
    }
}

impl Encode for VarInt {
    fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
        Self::encode(self, w)
    }
}

impl Decode<'_> for VarInt {
    fn decode(r: &mut &[u8]) -> ProtocolResult<Self> {
        Self::decode(r)
    }
}

impl From<i32> for VarInt {
    fn from(val: i32) -> Self {
        Self(val)
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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
        2_097_151,
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
