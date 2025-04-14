use crate::types::traits::{ProtocolRead, ProtocolWrite};
use crate::types::var_numbers::VarInt;
use std::io::{self, Read, Write};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteArray(pub Vec<u8>);

impl ProtocolWrite for ByteArray {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        let len = VarInt(self.0.len() as i32);
        let mut bytes_written = len.write_to(writer)?;
        writer.write_all(&self.0)?;
        bytes_written += self.0.len();
        Ok(bytes_written)
    }
}

impl ProtocolRead for ByteArray {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let (VarInt(length), mut bytes_read) = VarInt::read_from(reader)?;
        if length < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ByteArray length cannot be negative",
            ));
        }
        let mut buffer = vec![0u8; length as usize];
        reader.read_exact(&mut buffer)?;
        bytes_read += length as usize;
        Ok((ByteArray(buffer), bytes_read))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixedArray<T>(pub Vec<T>);

impl<T: ProtocolRead + ProtocolWrite> ProtocolWrite for PrefixedArray<T> {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        let mut written = VarInt(self.0.len() as i32).write_to(writer)?;
        for item in &self.0 {
            written += item.write_to(writer)?;
        }
        Ok(written)
    }
}

impl<T: ProtocolRead + ProtocolWrite> ProtocolRead for PrefixedArray<T> {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let (VarInt(length), mut bytes_read) = VarInt::read_from(reader)?;
        let mut items = Vec::with_capacity(length as usize);

        for _ in 0..length {
            let (item, n) = T::read_from(reader)?;
            bytes_read += n;
            items.push(item);
        }

        Ok((PrefixedArray(items), bytes_read))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::primitives::Int;
    use std::io::Cursor;

    #[test]
    fn test_byte_array() {
        let test_data = vec![1, 2, 3, 4, 5];
        let byte_array = ByteArray(test_data.clone());

        let mut buffer = Vec::new();
        let written = byte_array.write_to(&mut buffer).unwrap();

        let mut cursor = Cursor::new(buffer);
        let (read_array, read) = ByteArray::read_from(&mut cursor).unwrap();

        assert_eq!(written, read);
        assert_eq!(byte_array.0, read_array.0);
    }

    #[test]
    fn test_prefixed_array() {
        let test_data = vec![Int(1), Int(2), Int(3)];
        let array = PrefixedArray(test_data.clone());

        let mut buffer = Vec::new();
        let written = array.write_to(&mut buffer).unwrap();

        let mut cursor = Cursor::new(buffer);
        let (read_array, read) = PrefixedArray::<Int>::read_from(&mut cursor).unwrap();

        assert_eq!(written, read);
        assert_eq!(array.0, read_array.0);
    }

    #[test]
    fn test_empty_arrays() {
        // Test empty byte array
        let empty_bytes = ByteArray(vec![]);
        let mut buffer = Vec::new();
        empty_bytes.write_to(&mut buffer).unwrap();

        let mut cursor = Cursor::new(buffer);
        let (read_bytes, _) = ByteArray::read_from(&mut cursor).unwrap();
        assert!(read_bytes.0.is_empty());

        // Test empty prefixed array
        let empty_ints: PrefixedArray<Int> = PrefixedArray(vec![]);
        let mut buffer = Vec::new();
        empty_ints.write_to(&mut buffer).unwrap();

        let mut cursor = Cursor::new(buffer);
        let (read_ints, _) = PrefixedArray::<Int>::read_from(&mut cursor).unwrap();
        assert!(read_ints.0.is_empty());
    }
}
