use crate::types::traits::{ProtocolRead, ProtocolWrite};
use std::io::{self, Read, Write};

// Boolean type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Boolean(pub bool);

impl ProtocolWrite for Boolean {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        let byte = if self.0 { 0x01 } else { 0x00 };
        writer.write_all(&[byte])?;
        Ok(1)
    }
}

impl ProtocolRead for Boolean {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut buf = [0u8; 1];
        reader.read_exact(&mut buf)?;
        Ok((Boolean(buf[0] != 0), 1))
    }
}

// Byte type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Byte(pub i8);

impl ProtocolWrite for Byte {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write_all(&[self.0 as u8])?;
        Ok(1)
    }
}

impl ProtocolRead for Byte {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut buf = [0u8; 1];
        reader.read_exact(&mut buf)?;
        Ok((Byte(buf[0] as i8), 1))
    }
}

// Short type (signed 16-bit)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Short(pub i16);

impl ProtocolWrite for Short {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write_all(&self.0.to_be_bytes())?;
        Ok(2)
    }
}

impl ProtocolRead for Short {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut buf = [0u8; 2];
        reader.read_exact(&mut buf)?;
        Ok((Short(i16::from_be_bytes(buf)), 2))
    }
}

// Int type (signed 32-bit)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Int(pub i32);

impl ProtocolWrite for Int {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write_all(&self.0.to_be_bytes())?;
        Ok(4)
    }
}

impl ProtocolRead for Int {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf)?;
        Ok((Int(i32::from_be_bytes(buf)), 4))
    }
}

// Long type (signed 64-bit)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Long(pub i64);

impl ProtocolWrite for Long {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write_all(&self.0.to_be_bytes())?;
        Ok(8)
    }
}

impl ProtocolRead for Long {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut buf = [0u8; 8];
        reader.read_exact(&mut buf)?;
        Ok((Long(i64::from_be_bytes(buf)), 8))
    }
}

// Float type
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Float(pub f32);

impl ProtocolWrite for Float {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write_all(&self.0.to_be_bytes())?;
        Ok(4)
    }
}

impl ProtocolRead for Float {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf)?;
        Ok((Float(f32::from_be_bytes(buf)), 4))
    }
}

// Double type
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Double(pub f64);

impl ProtocolWrite for Double {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write_all(&self.0.to_be_bytes())?;
        Ok(8)
    }
}

impl ProtocolRead for Double {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut buf = [0u8; 8];
        reader.read_exact(&mut buf)?;
        Ok((Double(f64::from_be_bytes(buf)), 8))
    }
}

// UnsignedShort type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsignedShort(pub u16);

impl ProtocolWrite for UnsignedShort {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write_all(&self.0.to_be_bytes())?;
        Ok(2)
    }
}

impl ProtocolRead for UnsignedShort {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut buf = [0u8; 2];
        reader.read_exact(&mut buf)?;
        Ok((UnsignedShort(u16::from_be_bytes(buf)), 2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_boolean() {
        let test_cases = vec![true, false];
        for &value in &test_cases {
            let boolean = Boolean(value);
            let mut buffer = Vec::new();
            let written = boolean.write_to(&mut buffer).unwrap();

            let mut cursor = Cursor::new(buffer);
            let (read_value, read) = Boolean::read_from(&mut cursor).unwrap();

            assert_eq!(written, read);
            assert_eq!(boolean.0, read_value.0);
        }
    }

    #[test]
    fn test_numeric_types() {
        // Test Short
        let short = Short(12345);
        let mut buffer = Vec::new();
        short.write_to(&mut buffer).unwrap();
        let (read_short, _) = Short::read_from(&mut Cursor::new(buffer)).unwrap();
        assert_eq!(short.0, read_short.0);

        // Test Int
        let int = Int(1234567);
        let mut buffer = Vec::new();
        int.write_to(&mut buffer).unwrap();
        let (read_int, _) = Int::read_from(&mut Cursor::new(buffer)).unwrap();
        assert_eq!(int.0, read_int.0);

        // Test Float
        let float = Float(123.456);
        let mut buffer = Vec::new();
        float.write_to(&mut buffer).unwrap();
        let (read_float, _) = Float::read_from(&mut Cursor::new(buffer)).unwrap();
        assert_eq!(float.0, read_float.0);
    }
}
