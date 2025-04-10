use crate::network::packet::{Packet, PacketCodec};
use crate::protocol::types::{ByteArray, ProtocolRead, ProtocolWrite};
use std::convert::TryFrom;
use std::io;

pub const SERVERBOUND_ENCRYPTION_RESPONSE_ID: i32 = 0x01;

#[derive(Debug, Clone, PartialEq)]
pub struct ServerBoundEncryptionResponse {
    pub shared_secret: ByteArray,
    pub verify_token: ByteArray,
}

impl ServerBoundEncryptionResponse {
    pub fn new(shared_secret: Vec<u8>, verify_token: Vec<u8>) -> Self {
        Self {
            shared_secret: ByteArray(shared_secret),
            verify_token: ByteArray(verify_token),
        }
    }
}

impl ProtocolWrite for ServerBoundEncryptionResponse {
    fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let mut written = 0;
        written += self.shared_secret.write_to(writer)?;
        written += self.verify_token.write_to(writer)?;
        Ok(written)
    }
}

impl ProtocolRead for ServerBoundEncryptionResponse {
    fn read_from<R: io::Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut bytes_read = 0;

        let (shared_secret, n) = ByteArray::read_from(reader)?;
        bytes_read += n;

        let (verify_token, n) = ByteArray::read_from(reader)?;
        bytes_read += n;

        Ok((
            Self {
                shared_secret,
                verify_token,
            },
            bytes_read,
        ))
    }
}

impl TryFrom<&Packet> for ServerBoundEncryptionResponse {
    type Error = io::Error;

    fn try_from(packet: &Packet) -> Result<Self, Self::Error> {
        if packet.id != SERVERBOUND_ENCRYPTION_RESPONSE_ID {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid packet ID for encryption response",
            ));
        }
        packet
            .decode()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_encryption_response() {
        let response = ServerBoundEncryptionResponse::new(vec![1, 2, 3, 4], vec![5, 6, 7, 8]);

        let mut buffer = Vec::new();
        let written = response.write_to(&mut buffer).unwrap();

        let mut cursor = Cursor::new(buffer);
        let (read_response, read) = ServerBoundEncryptionResponse::read_from(&mut cursor).unwrap();

        assert_eq!(written, read);
        assert_eq!(response, read_response);
    }
}
