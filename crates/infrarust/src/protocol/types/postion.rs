use std::io::{self, Read, Write};
use crate::protocol::types::traits::{ProtocolRead, ProtocolWrite};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub x: i32, // 26 bits
    pub y: i16, // 12 bits
    pub z: i32, // 26 bits
}

impl Position {
    pub fn new(x: i32, y: i16, z: i32) -> Self {
        Position { x, y, z }
    }

    pub fn encode(&self) -> i64 {
        ((self.x as i64 & 0x3FFFFFF) << 38)
            | ((self.z as i64 & 0x3FFFFFF) << 12)
            | (self.y as i64 & 0xFFF)
    }

    pub fn decode(value: i64) -> Self {
        let x = (value >> 38) as i32;
        let y = ((value << 52) >> 52) as i16;
        let z = ((value << 26) >> 38) as i32;
        Position { x, y, z }
    }
}

impl ProtocolWrite for Position {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        let encoded = self.encode();
        writer.write_all(&encoded.to_be_bytes())?;
        Ok(8)
    }
}

impl ProtocolRead for Position {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut buf = [0u8; 8];
        reader.read_exact(&mut buf)?;
        let value = i64::from_be_bytes(buf);
        Ok((Position::decode(value), 8))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_position_encoding() {
        let test_cases = vec![
            Position::new(0, 0, 0),
            Position::new(18357644, 831, -20882616),
            Position::new(-1, -1, -1),
            Position::new(i32::MAX >> 6, i16::MAX >> 4, i32::MAX >> 6),
        ];

        for pos in test_cases {
            let encoded = pos.encode();
            let decoded = Position::decode(encoded);
            assert_eq!(pos, decoded);
        }
    }

    #[test]
    fn test_position_protocol() {
        let pos = Position::new(18357644, 831, -20882616);
        let mut buffer = Vec::new();
        let written = pos.write_to(&mut buffer).unwrap();
        
        let mut cursor = Cursor::new(buffer);
        let (read_pos, read) = Position::read_from(&mut cursor).unwrap();
        
        assert_eq!(written, read);
        assert_eq!(pos, read_pos);
    }

    #[test]
    fn test_position_bounds() {
        // Test max values (26 bits for x/z, 12 bits for y)
        let max_pos = Position::new(
            (1 << 25) - 1,  // Max 26-bit signed value
            (1 << 11) - 1,  // Max 12-bit signed value
            (1 << 25) - 1   // Max 26-bit signed value
        );
        
        let encoded = max_pos.encode();
        let decoded = Position::decode(encoded);
        assert_eq!(max_pos, decoded);
    }
}