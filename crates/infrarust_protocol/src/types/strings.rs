use crate::protocol::types::traits::{ProtocolRead, ProtocolWrite};
use crate::protocol::types::var_numbers::VarInt;
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolString(pub String);

impl ProtocolWrite for ProtocolString {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        let bytes = self.0.as_bytes();
        let len = VarInt(bytes.len() as i32);
        let mut bytes_written = len.write_to(writer)?;
        writer.write_all(bytes)?;
        bytes_written += bytes.len();
        Ok(bytes_written)
    }
}

impl ProtocolRead for ProtocolString {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let (VarInt(length), mut bytes_read) = VarInt::read_from(reader)?;
        if length < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "String length cannot be negative",
            ));
        }
        let mut buffer = vec![0u8; length as usize];
        reader.read_exact(&mut buffer)?;
        bytes_read += length as usize;

        let string =
            String::from_utf8(buffer).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        Ok((ProtocolString(string), bytes_read))
    }
}

// Chat type alias
pub type Chat = ProtocolString;

// Identifier type for Minecraft resource names
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identifier(pub String);

impl Identifier {
    fn is_valid_namespace(s: &str) -> bool {
        !s.is_empty()
            && s.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-' || c == '_'
            })
    }

    fn is_valid_path(s: &str) -> bool {
        !s.is_empty()
            && s.chars().all(|c| {
                c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || c == '.'
                    || c == '-'
                    || c == '_'
                    || c == '/'
            })
    }
}

impl ProtocolWrite for Identifier {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        ProtocolString(self.0.clone()).write_to(writer)
    }
}

impl ProtocolRead for Identifier {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let (ProtocolString(value), size) = ProtocolString::read_from(reader)?;

        let (namespace, path) = match value.split_once(':') {
            Some((ns, p)) => (ns, p),
            None => ("minecraft", value.as_str()), // Default namespace
        };

        if !Self::is_valid_namespace(namespace) || !Self::is_valid_path(path) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid Minecraft identifier format",
            ));
        }

        Ok((Identifier(value), size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_protocol_string() {
        let test_cases = vec!["", "Hello World!", "Test string with unicode 🦀"];

        for test_str in test_cases {
            let protocol_string = ProtocolString(test_str.to_string());
            let mut buffer = Vec::new();
            let written = protocol_string.write_to(&mut buffer).unwrap();

            let mut cursor = Cursor::new(buffer);
            let (read_string, read) = ProtocolString::read_from(&mut cursor).unwrap();

            assert_eq!(written, read);
            assert_eq!(protocol_string.0, read_string.0);
        }
    }

    #[test]
    fn test_identifier() {
        let valid_ids = vec![
            "minecraft:stone",
            "custom:special_block",
            "test:path/to/resource",
        ];

        let invalid_ids = vec![
            "Invalid:uppercase",
            "contains:invalid@chars",
            ":no_namespace",
        ];

        for id in valid_ids {
            let identifier = Identifier(id.to_string());
            let mut buffer = Vec::new();
            identifier.write_to(&mut buffer).unwrap();

            let mut cursor = Cursor::new(buffer);
            let result = Identifier::read_from(&mut cursor);
            assert!(result.is_ok());
        }

        for id in invalid_ids {
            let identifier = Identifier(id.to_string());
            let mut buffer = Vec::new();
            identifier.write_to(&mut buffer).unwrap();

            let mut cursor = Cursor::new(buffer);
            let result = Identifier::read_from(&mut cursor);
            assert!(result.is_err());
        }
    }
}
