pub mod types;
pub mod varint;
pub mod varlong;

pub use varint::VarInt;
pub use varlong::VarLong;

use std::io::{Read, Write};

use crate::error::ProtocolResult;

pub trait Encode {
    fn encode(&self, w: &mut impl Write) -> ProtocolResult<()>;
}

pub trait Decode<'a>: Sized {
    fn decode(r: &mut &'a [u8]) -> ProtocolResult<Self>;
}

#[allow(clippy::missing_errors_doc)]
pub trait McBufReadExt: Read {
    fn read_u8(&mut self) -> ProtocolResult<u8>;
    fn read_i8(&mut self) -> ProtocolResult<i8>;
    fn read_u16_be(&mut self) -> ProtocolResult<u16>;
    fn read_i16_be(&mut self) -> ProtocolResult<i16>;
    fn read_u32_be(&mut self) -> ProtocolResult<u32>;
    fn read_i32_be(&mut self) -> ProtocolResult<i32>;
    fn read_u64_be(&mut self) -> ProtocolResult<u64>;
    fn read_i64_be(&mut self) -> ProtocolResult<i64>;
    fn read_u128_be(&mut self) -> ProtocolResult<u128>;
    fn read_f32_be(&mut self) -> ProtocolResult<f32>;
    fn read_f64_be(&mut self) -> ProtocolResult<f64>;
    fn read_bool(&mut self) -> ProtocolResult<bool>;
    fn read_var_int(&mut self) -> ProtocolResult<VarInt>;
    fn read_var_long(&mut self) -> ProtocolResult<VarLong>;
    fn read_string(&mut self) -> ProtocolResult<String>;
    fn read_string_bounded(&mut self, max_len: usize) -> ProtocolResult<String>;
    fn read_uuid(&mut self) -> ProtocolResult<uuid::Uuid>;
    fn read_byte_array(&mut self, max_len: usize) -> ProtocolResult<Vec<u8>>;
    fn read_byte_array_bounded(&mut self, count: usize) -> ProtocolResult<Vec<u8>>;
    fn read_remaining(&mut self) -> ProtocolResult<Vec<u8>>;
}

#[allow(clippy::missing_errors_doc)]
pub trait McBufWriteExt: Write {
    fn write_u8(&mut self, value: u8) -> ProtocolResult<()>;
    fn write_i8(&mut self, value: i8) -> ProtocolResult<()>;
    fn write_u16_be(&mut self, value: u16) -> ProtocolResult<()>;
    fn write_i16_be(&mut self, value: i16) -> ProtocolResult<()>;
    fn write_u32_be(&mut self, value: u32) -> ProtocolResult<()>;
    fn write_i32_be(&mut self, value: i32) -> ProtocolResult<()>;
    fn write_u64_be(&mut self, value: u64) -> ProtocolResult<()>;
    fn write_i64_be(&mut self, value: i64) -> ProtocolResult<()>;
    fn write_u128_be(&mut self, value: u128) -> ProtocolResult<()>;
    fn write_f32_be(&mut self, value: f32) -> ProtocolResult<()>;
    fn write_f64_be(&mut self, value: f64) -> ProtocolResult<()>;
    fn write_bool(&mut self, value: bool) -> ProtocolResult<()>;
    fn write_var_int(&mut self, value: &VarInt) -> ProtocolResult<()>;
    fn write_var_long(&mut self, value: &VarLong) -> ProtocolResult<()>;
    fn write_string(&mut self, value: &str) -> ProtocolResult<()>;
    fn write_uuid(&mut self, value: &uuid::Uuid) -> ProtocolResult<()>;
    fn write_byte_array(&mut self, data: &[u8]) -> ProtocolResult<()>;
}

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
        types::bool_from_byte(self.read_u8()?)
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

    fn write_byte_array(&mut self, data: &[u8]) -> ProtocolResult<()> {
        VarInt(data.len() as i32).encode(self)?;
        self.write_all(data)?;
        Ok(())
    }
}
