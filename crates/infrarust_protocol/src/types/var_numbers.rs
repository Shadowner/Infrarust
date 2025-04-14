use crate::types::traits::{ProtocolRead, ProtocolWrite, WriteToBytes};
use bytes::{BufMut, BytesMut};
use std::io::{self, Read, Write};

const SEGMENT_BITS: i32 = 0x7F;
const _CONTINUE_BIT: i32 = 0x80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VarInt(pub i32);

impl VarInt {
    pub fn exceeds_three_bytes(&self) -> bool {
        let value = if self.0 >= 0 {
            self.0 as u32
        } else {
            (self.0 as u32).wrapping_neg()
        };
        value >= 0x200000 // 2^21
    }

    pub fn len(&self) -> usize {
        let mut value = if self.0 >= 0 {
            self.0 as u32
        } else {
            (self.0 as u32).wrapping_neg()
        };

        let mut size = 0;
        loop {
            size += 1;
            if value <= 0x7f {
                break;
            }
            value >>= 7;
            if size >= 5 {
                break;
            }
        }
        size
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    pub fn to_bytes(&self) -> BytesMut {
        let mut buffer = BytesMut::with_capacity(5);
        let mut value = self.0 as u32;

        loop {
            let mut byte = (value & 0x7F) as u8;
            value >>= 7;

            if value != 0 {
                byte |= 0x80;
            }

            buffer.extend_from_slice(&[byte]);

            if value == 0 {
                break;
            }
        }

        buffer
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VarLong(pub i64);

impl VarLong {
    pub fn len(&self) -> usize {
        let mut value = self.0 as u64;
        let mut size = 0;

        loop {
            size += 1;
            if (value & !(SEGMENT_BITS as u64)) == 0 {
                break;
            }
            value >>= 7;
            if size >= 10 {
                break;
            }
        }
        size
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }
}

impl ProtocolWrite for VarInt {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        if self.exceeds_three_bytes() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "VarInt too large (exceeds 3 bytes)",
            ));
        }

        let mut value = self.0 as u32;
        let mut bytes_written = 0;

        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;

            if value != 0 {
                byte |= 0x80;
            }

            writer.write_all(&[byte])?;
            bytes_written += 1;

            if value == 0 {
                break;
            }
        }

        Ok(bytes_written)
    }
}

impl ProtocolRead for VarInt {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut value: i32 = 0;
        let mut position = 0;
        let mut bytes_read = 0;

        loop {
            if bytes_read >= 5 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "VarInt too long (>5 bytes)",
                ));
            }

            let mut buf = [0u8; 1];
            reader.read_exact(&mut buf)?;
            bytes_read += 1;

            let byte = buf[0];
            value |= ((byte & 0x7f) as i32) << position;

            if byte & 0x80 == 0 {
                break;
            }

            position += 7;
            if position >= 32 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "VarInt exceeds 32 bits",
                ));
            }
        }

        Ok((VarInt(value), bytes_read))
    }
}

impl WriteToBytes for VarInt {
    fn write_to_bytes(&self, bytes: &mut BytesMut) -> io::Result<usize> {
        if self.exceeds_three_bytes() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "VarInt too large (exceeds 3 bytes)",
            ));
        }

        let mut value = self.0 as u32;
        let mut bytes_written = 0;

        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;

            if value != 0 {
                byte |= 0x80;
            }

            bytes.put_u8(byte);
            bytes_written += 1;

            if value == 0 {
                break;
            }
        }

        Ok(bytes_written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_varint_write_read() {
        let test_values = vec![
            0, 1, 127, 128, 255, 2097151, // Maximum 3-byte value
        ];

        for value in test_values {
            let varint = VarInt(value);
            let mut buffer = Vec::new();
            let written = varint.write_to(&mut buffer).unwrap();
            assert!(written <= 3, "VarInt encoded with more than 3 bytes");

            let mut cursor = Cursor::new(buffer);
            let (read_varint, read) = VarInt::read_from(&mut cursor).unwrap();

            assert_eq!(written, read);
            assert_eq!(varint.0, read_varint.0);
        }
    }

    #[test]
    #[should_panic(expected = "VarInt too large")]
    fn test_varint_too_large() {
        let large_value = VarInt(2097152); // First value requiring 4 bytes
        let mut buffer = Vec::new();
        large_value.write_to(&mut buffer).unwrap();
    }
}
