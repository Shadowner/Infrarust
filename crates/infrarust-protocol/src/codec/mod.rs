//! Minecraft protocol codec: encoding/decoding traits and primitive type implementations.
//!
//! # Architecture
//!
//! - [`Encode`] / [`Decode`] — Core traits for wire-format serialization.
//! - [`McBufReadExt`] / [`McBufWriteExt`] — Extension traits adding Minecraft read/write
//!   methods to any `impl Read` / `impl Write`.
//! - [`VarInt`] / [`VarLong`] — Variable-length integer types used throughout the protocol.
//! - [`types`] — Encode/Decode implementations for all Minecraft primitive types.

pub mod types;
pub mod varint;
pub mod varlong;

pub use varint::VarInt;
pub use varlong::VarLong;

use std::io::{Read, Write};

use crate::error::ProtocolResult;

/// Encodes a type into the Minecraft wire format.
///
/// For stable primitive types (`VarInt`, String, UUID, etc.) that don't vary between
/// protocol versions. Version-dependent packets use the `Packet` trait (defined in
/// a later phase) which takes a `ProtocolVersion` parameter.
///
/// Design inspired by Valence (`valence_protocol`):
/// - `impl Write` allows writing to `Vec<u8>`, `BytesMut`, or any other writer.
/// - No lifetimes needed since encoding never borrows.
pub trait Encode {
    /// Writes this value into `w` in Minecraft wire format.
    fn encode(&self, w: &mut impl Write) -> ProtocolResult<()>;
}

/// Decodes a type from the Minecraft wire format.
///
/// The `&mut &'a [u8]` (double reference) pattern comes from Valence:
/// - The outer `&mut` allows advancing the cursor.
/// - The inner `&'a [u8]` enables zero-copy borrows from the buffer.
/// - After a successful decode, `r` points to the remaining bytes.
///
/// # Example
///
/// ```
/// # use infrarust_protocol::codec::Decode;
/// let mut buf: &[u8] = &[0x01, 0x02, 0x03];
/// let val = u8::decode(&mut buf).unwrap(); // val = 1, buf = &[0x02, 0x03]
/// assert_eq!(val, 1);
/// assert_eq!(buf, &[0x02, 0x03]);
/// ```
pub trait Decode<'a>: Sized {
    /// Reads a value from `r` in Minecraft wire format, advancing the cursor.
    fn decode(r: &mut &'a [u8]) -> ProtocolResult<Self>;
}

/// Minecraft read methods added to any `impl Read`.
pub trait McBufReadExt: Read {
    /// Reads a single unsigned byte.
    fn read_u8(&mut self) -> ProtocolResult<u8>;
    /// Reads a single signed byte.
    fn read_i8(&mut self) -> ProtocolResult<i8>;
    /// Reads an unsigned 16-bit integer in big-endian.
    fn read_u16_be(&mut self) -> ProtocolResult<u16>;
    /// Reads a signed 16-bit integer in big-endian.
    fn read_i16_be(&mut self) -> ProtocolResult<i16>;
    /// Reads an unsigned 32-bit integer in big-endian.
    fn read_u32_be(&mut self) -> ProtocolResult<u32>;
    /// Reads a signed 32-bit integer in big-endian.
    fn read_i32_be(&mut self) -> ProtocolResult<i32>;
    /// Reads an unsigned 64-bit integer in big-endian.
    fn read_u64_be(&mut self) -> ProtocolResult<u64>;
    /// Reads a signed 64-bit integer in big-endian.
    fn read_i64_be(&mut self) -> ProtocolResult<i64>;
    /// Reads an unsigned 128-bit integer in big-endian.
    fn read_u128_be(&mut self) -> ProtocolResult<u128>;
    /// Reads a 32-bit float in big-endian.
    fn read_f32_be(&mut self) -> ProtocolResult<f32>;
    /// Reads a 64-bit float in big-endian.
    fn read_f64_be(&mut self) -> ProtocolResult<f64>;
    /// Reads a boolean (single byte, 0 or 1).
    fn read_bool(&mut self) -> ProtocolResult<bool>;
    /// Reads a `VarInt`.
    fn read_var_int(&mut self) -> ProtocolResult<VarInt>;
    /// Reads a `VarLong`.
    fn read_var_long(&mut self) -> ProtocolResult<VarLong>;
    /// Reads a `VarInt`-prefixed UTF-8 string (max 32767 chars).
    fn read_string(&mut self) -> ProtocolResult<String>;
    /// Reads a `VarInt`-prefixed UTF-8 string with a custom character limit.
    fn read_string_bounded(&mut self, max_len: usize) -> ProtocolResult<String>;
    /// Reads a UUID (16 bytes big-endian).
    fn read_uuid(&mut self) -> ProtocolResult<uuid::Uuid>;
    /// Reads a `VarInt`-prefixed byte array with a maximum length.
    fn read_byte_array(&mut self, max_len: usize) -> ProtocolResult<Vec<u8>>;
    /// Reads exactly `count` bytes.
    fn read_byte_array_bounded(&mut self, count: usize) -> ProtocolResult<Vec<u8>>;
    /// Reads all remaining bytes.
    fn read_remaining(&mut self) -> ProtocolResult<Vec<u8>>;
}

/// Minecraft write methods added to any `impl Write`.
pub trait McBufWriteExt: Write {
    /// Writes a single unsigned byte.
    fn write_u8(&mut self, value: u8) -> ProtocolResult<()>;
    /// Writes a single signed byte.
    fn write_i8(&mut self, value: i8) -> ProtocolResult<()>;
    /// Writes an unsigned 16-bit integer in big-endian.
    fn write_u16_be(&mut self, value: u16) -> ProtocolResult<()>;
    /// Writes a signed 16-bit integer in big-endian.
    fn write_i16_be(&mut self, value: i16) -> ProtocolResult<()>;
    /// Writes an unsigned 32-bit integer in big-endian.
    fn write_u32_be(&mut self, value: u32) -> ProtocolResult<()>;
    /// Writes a signed 32-bit integer in big-endian.
    fn write_i32_be(&mut self, value: i32) -> ProtocolResult<()>;
    /// Writes an unsigned 64-bit integer in big-endian.
    fn write_u64_be(&mut self, value: u64) -> ProtocolResult<()>;
    /// Writes a signed 64-bit integer in big-endian.
    fn write_i64_be(&mut self, value: i64) -> ProtocolResult<()>;
    /// Writes an unsigned 128-bit integer in big-endian.
    fn write_u128_be(&mut self, value: u128) -> ProtocolResult<()>;
    /// Writes a 32-bit float in big-endian.
    fn write_f32_be(&mut self, value: f32) -> ProtocolResult<()>;
    /// Writes a 64-bit float in big-endian.
    fn write_f64_be(&mut self, value: f64) -> ProtocolResult<()>;
    /// Writes a boolean (single byte).
    fn write_bool(&mut self, value: bool) -> ProtocolResult<()>;
    /// Writes a `VarInt`.
    fn write_var_int(&mut self, value: &VarInt) -> ProtocolResult<()>;
    /// Writes a `VarLong`.
    fn write_var_long(&mut self, value: &VarLong) -> ProtocolResult<()>;
    /// Writes a `VarInt`-prefixed UTF-8 string.
    fn write_string(&mut self, value: &str) -> ProtocolResult<()>;
    /// Writes a UUID (16 bytes big-endian).
    fn write_uuid(&mut self, value: &uuid::Uuid) -> ProtocolResult<()>;
    /// Writes a `VarInt`-prefixed byte array.
    fn write_byte_array(&mut self, data: &[u8]) -> ProtocolResult<()>;
}

// --- Blanket implementation for McBufReadExt ---

impl<R: Read> McBufReadExt for R {
    fn read_u8(&mut self) -> ProtocolResult<u8> {
        let mut buf = [0u8; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    fn read_i8(&mut self) -> ProtocolResult<i8> {
        Ok(self.read_u8()?.cast_signed())
    }

    fn read_u16_be(&mut self) -> ProtocolResult<u16> {
        let mut buf = [0u8; 2];
        self.read_exact(&mut buf)?;
        Ok(u16::from_be_bytes(buf))
    }

    fn read_i16_be(&mut self) -> ProtocolResult<i16> {
        let mut buf = [0u8; 2];
        self.read_exact(&mut buf)?;
        Ok(i16::from_be_bytes(buf))
    }

    fn read_u32_be(&mut self) -> ProtocolResult<u32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(u32::from_be_bytes(buf))
    }

    fn read_i32_be(&mut self) -> ProtocolResult<i32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(i32::from_be_bytes(buf))
    }

    fn read_u64_be(&mut self) -> ProtocolResult<u64> {
        let mut buf = [0u8; 8];
        self.read_exact(&mut buf)?;
        Ok(u64::from_be_bytes(buf))
    }

    fn read_i64_be(&mut self) -> ProtocolResult<i64> {
        let mut buf = [0u8; 8];
        self.read_exact(&mut buf)?;
        Ok(i64::from_be_bytes(buf))
    }

    fn read_u128_be(&mut self) -> ProtocolResult<u128> {
        let mut buf = [0u8; 16];
        self.read_exact(&mut buf)?;
        Ok(u128::from_be_bytes(buf))
    }

    fn read_f32_be(&mut self) -> ProtocolResult<f32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(f32::from_be_bytes(buf))
    }

    fn read_f64_be(&mut self) -> ProtocolResult<f64> {
        let mut buf = [0u8; 8];
        self.read_exact(&mut buf)?;
        Ok(f64::from_be_bytes(buf))
    }

    fn read_bool(&mut self) -> ProtocolResult<bool> {
        let byte = self.read_u8()?;
        match byte {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(crate::error::ProtocolError::invalid(
                "bool value must be 0 or 1",
            )),
        }
    }

    fn read_var_int(&mut self) -> ProtocolResult<VarInt> {
        types::read_varint_from_reader(self)
    }

    fn read_var_long(&mut self) -> ProtocolResult<VarLong> {
        types::read_varlong_from_reader(self)
    }

    fn read_string(&mut self) -> ProtocolResult<String> {
        self.read_string_bounded(32767)
    }

    fn read_string_bounded(&mut self, max_len: usize) -> ProtocolResult<String> {
        types::read_string_bounded_from_reader(self, max_len)
    }

    fn read_uuid(&mut self) -> ProtocolResult<uuid::Uuid> {
        let val = self.read_u128_be()?;
        Ok(uuid::Uuid::from_u128(val))
    }

    fn read_byte_array(&mut self, max_len: usize) -> ProtocolResult<Vec<u8>> {
        let raw_len = self.read_var_int()?.0;
        if raw_len < 0 {
            return Err(crate::error::ProtocolError::invalid("negative length"));
        }
        #[allow(clippy::cast_sign_loss)] // Checked non-negative above
        let len = raw_len as usize;
        if len > max_len {
            return Err(crate::error::ProtocolError::too_large(max_len, len));
        }
        self.read_byte_array_bounded(len)
    }

    fn read_byte_array_bounded(&mut self, count: usize) -> ProtocolResult<Vec<u8>> {
        let mut buf = vec![0u8; count];
        self.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn read_remaining(&mut self) -> ProtocolResult<Vec<u8>> {
        let mut buf = Vec::new();
        self.read_to_end(&mut buf)?;
        Ok(buf)
    }
}

// --- Blanket implementation for McBufWriteExt ---

impl<W: Write> McBufWriteExt for W {
    fn write_u8(&mut self, value: u8) -> ProtocolResult<()> {
        self.write_all(&[value])?;
        Ok(())
    }

    fn write_i8(&mut self, value: i8) -> ProtocolResult<()> {
        self.write_all(&[value.cast_unsigned()])?;
        Ok(())
    }

    fn write_u16_be(&mut self, value: u16) -> ProtocolResult<()> {
        self.write_all(&value.to_be_bytes())?;
        Ok(())
    }

    fn write_i16_be(&mut self, value: i16) -> ProtocolResult<()> {
        self.write_all(&value.to_be_bytes())?;
        Ok(())
    }

    fn write_u32_be(&mut self, value: u32) -> ProtocolResult<()> {
        self.write_all(&value.to_be_bytes())?;
        Ok(())
    }

    fn write_i32_be(&mut self, value: i32) -> ProtocolResult<()> {
        self.write_all(&value.to_be_bytes())?;
        Ok(())
    }

    fn write_u64_be(&mut self, value: u64) -> ProtocolResult<()> {
        self.write_all(&value.to_be_bytes())?;
        Ok(())
    }

    fn write_i64_be(&mut self, value: i64) -> ProtocolResult<()> {
        self.write_all(&value.to_be_bytes())?;
        Ok(())
    }

    fn write_u128_be(&mut self, value: u128) -> ProtocolResult<()> {
        self.write_all(&value.to_be_bytes())?;
        Ok(())
    }

    fn write_f32_be(&mut self, value: f32) -> ProtocolResult<()> {
        self.write_all(&value.to_be_bytes())?;
        Ok(())
    }

    fn write_f64_be(&mut self, value: f64) -> ProtocolResult<()> {
        self.write_all(&value.to_be_bytes())?;
        Ok(())
    }

    fn write_bool(&mut self, value: bool) -> ProtocolResult<()> {
        self.write_all(&[u8::from(value)])?;
        Ok(())
    }

    fn write_var_int(&mut self, value: &VarInt) -> ProtocolResult<()> {
        value.encode(self)
    }

    fn write_var_long(&mut self, value: &VarLong) -> ProtocolResult<()> {
        value.encode(self)
    }

    fn write_string(&mut self, value: &str) -> ProtocolResult<()> {
        types::encode_string(value, self)
    }

    fn write_uuid(&mut self, value: &uuid::Uuid) -> ProtocolResult<()> {
        self.write_all(&value.as_u128().to_be_bytes())?;
        Ok(())
    }

    #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)] // Byte array length bounded by protocol limits
    fn write_byte_array(&mut self, data: &[u8]) -> ProtocolResult<()> {
        VarInt(data.len() as i32).encode(self)?;
        self.write_all(data)?;
        Ok(())
    }
}
