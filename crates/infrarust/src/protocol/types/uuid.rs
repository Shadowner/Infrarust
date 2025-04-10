use crate::protocol::types::traits::{ProtocolRead, ProtocolWrite};
use std::io::{self, Read, Write};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolUUID(pub Uuid);

impl ProtocolWrite for ProtocolUUID {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write_all(self.0.as_bytes())?;
        Ok(16)
    }
}

impl ProtocolRead for ProtocolUUID {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut buffer = [0u8; 16];
        reader.read_exact(&mut buffer)?;
        Ok((ProtocolUUID(Uuid::from_bytes(buffer)), 16))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_uuid_protocol() {
        let test_cases = vec![
            Uuid::nil(),
            Uuid::new_v4(),
            Uuid::parse_str("67e55044-10b1-426f-9247-bb680e5fe0c8").unwrap(),
        ];

        for uuid in test_cases {
            let protocol_uuid = ProtocolUUID(uuid);
            let mut buffer = Vec::new();
            let written = protocol_uuid.write_to(&mut buffer).unwrap();

            let mut cursor = Cursor::new(buffer);
            let (read_uuid, read) = ProtocolUUID::read_from(&mut cursor).unwrap();

            assert_eq!(written, read);
            assert_eq!(protocol_uuid, read_uuid);
        }
    }

    #[test]
    fn test_uuid_size() {
        let uuid = ProtocolUUID(Uuid::nil());
        let mut buffer = Vec::new();
        let written = uuid.write_to(&mut buffer).unwrap();
        assert_eq!(written, 16);
        assert_eq!(buffer.len(), 16);
    }
}
