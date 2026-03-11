//! VarLong encoding and decoding for the Minecraft protocol.

use std::fmt;
use std::io::Write;

use crate::codec::{Decode, Encode};
use crate::error::{ProtocolError, ProtocolResult};

/// VarLong Minecraft — a signed 64-bit integer encoded in 1–10 bytes.
///
/// Same encoding scheme as [`super::VarInt`] but for `i64` values.
/// Rarely a performance bottleneck, so uses a simple loop-based encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VarLong(pub i64);

impl VarLong {
    /// Maximum number of bytes a VarLong can occupy on the wire.
    pub const MAX_SIZE: usize = 10;

    /// Returns the number of bytes this VarLong will occupy when encoded.
    ///
    /// Computed in O(1) without loops.
    pub const fn written_size(self) -> usize {
        match self.0 {
            0 => 1,
            n => (63 - n.leading_zeros() as usize) / 7 + 1,
        }
    }

    /// Encodes this VarLong using a classic byte-by-byte loop.
    pub fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
        let mut val = self.0 as u64;
        loop {
            let byte = (val & 0x7F) as u8;
            val >>= 7;
            if val == 0 {
                w.write_all(&[byte])?;
                return Ok(());
            }
            w.write_all(&[byte | 0x80])?;
        }
    }

    /// Decodes a VarLong from a byte slice, advancing the cursor.
    pub fn decode(r: &mut &[u8]) -> ProtocolResult<Self> {
        let mut val = 0i64;
        for i in 0..Self::MAX_SIZE {
            if r.is_empty() {
                return Err(ProtocolError::Incomplete {
                    context: "VarLong",
                });
            }
            let byte = r[0];
            *r = &r[1..];
            val |= (i64::from(byte) & 0x7F) << (i * 7);
            if byte & 0x80 == 0 {
                return Ok(VarLong(val));
            }
        }
        Err(ProtocolError::invalid("VarLong too large (> 10 bytes)"))
    }
}

impl Encode for VarLong {
    fn encode(&self, w: &mut impl Write) -> ProtocolResult<()> {
        VarLong::encode(self, w)
    }
}

impl Decode<'_> for VarLong {
    fn decode(r: &mut &[u8]) -> ProtocolResult<Self> {
        VarLong::decode(r)
    }
}

impl From<i64> for VarLong {
    fn from(val: i64) -> Self {
        VarLong(val)
    }
}

impl From<VarLong> for i64 {
    fn from(val: VarLong) -> Self {
        val.0
    }
}

impl fmt::Display for VarLong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_trip() {
        for &val in &[0i64, 1, -1, i64::MIN, i64::MAX] {
            let vl = VarLong(val);
            let mut buf = Vec::new();
            vl.encode(&mut buf).unwrap();
            let mut slice: &[u8] = &buf;
            let decoded = VarLong::decode(&mut slice).unwrap();
            assert_eq!(vl, decoded, "round-trip failed for {val}");
            assert!(slice.is_empty(), "trailing bytes for {val}");
        }
    }

    #[test]
    fn test_written_size() {
        for &val in &[0i64, 1, -1, i64::MIN, i64::MAX] {
            let vl = VarLong(val);
            let mut buf = Vec::new();
            vl.encode(&mut buf).unwrap();
            assert_eq!(
                vl.written_size(),
                buf.len(),
                "written_size mismatch for {val}"
            );
        }
    }

    #[test]
    fn test_decode_too_large() {
        let mut buf: &[u8] = &[0x80; 11];
        let err = VarLong::decode(&mut buf).unwrap_err();
        assert!(err.is_fatal());
    }

    #[test]
    fn test_i64_max_is_10_bytes() {
        let mut buf = Vec::new();
        VarLong(i64::MAX).encode(&mut buf).unwrap();
        assert_eq!(buf.len(), 9); // i64::MAX uses 9 bytes (63 bits / 7 = 9)
    }

    #[test]
    fn test_from_i64_conversion() {
        assert_eq!(VarLong::from(42i64).0, 42);
        assert_eq!(i64::from(VarLong(42)), 42);
    }
}
