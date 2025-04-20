use crate::types::{Boolean, ByteArray, ProtocolRead, ProtocolString, ProtocolWrite};
use std::io;

pub const CLIENTBOUND_ENCRYPTION_REQUEST_ID: i32 = 0x01;

#[derive(Debug, Clone, PartialEq)]
pub struct ClientBoundEncryptionRequest {
    pub server_id: ProtocolString,
    pub public_key: ByteArray,
    pub verify_token: ByteArray,
    pub requires_authentication: Boolean,
}

impl ClientBoundEncryptionRequest {
    pub fn new(
        server_id: String,
        public_key: Vec<u8>,
        verify_token: Vec<u8>,
        requires_authentication: bool,
    ) -> Self {
        Self {
            server_id: ProtocolString(server_id),
            public_key: ByteArray(public_key),
            verify_token: ByteArray(verify_token),
            requires_authentication: Boolean(requires_authentication),
        }
    }
}

impl ProtocolWrite for ClientBoundEncryptionRequest {
    fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let mut written = 0;
        written += self.server_id.write_to(writer)?;
        written += self.public_key.write_to(writer)?;
        written += self.verify_token.write_to(writer)?;
        written += self.requires_authentication.write_to(writer)?;
        Ok(written)
    }
}

impl ProtocolRead for ClientBoundEncryptionRequest {
    fn read_from<R: io::Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut bytes_read = 0;

        let (server_id, n) = ProtocolString::read_from(reader)?;
        bytes_read += n;

        let (public_key, n) = ByteArray::read_from(reader)?;
        bytes_read += n;

        let (verify_token, n) = ByteArray::read_from(reader)?;
        bytes_read += n;

        let (requires_authentication, n) = Boolean::read_from(reader)?;
        bytes_read += n;

        Ok((
            Self {
                server_id,
                public_key,
                verify_token,
                requires_authentication,
            },
            bytes_read,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_encryption_request() {
        let request = ClientBoundEncryptionRequest::new(
            "test_server".to_string(),
            vec![1, 2, 3, 4],
            vec![5, 6, 7, 8],
            true,
        );

        let mut buffer = Vec::new();
        let written = request.write_to(&mut buffer).unwrap();

        let mut cursor = Cursor::new(buffer);
        let (read_request, read) = ClientBoundEncryptionRequest::read_from(&mut cursor).unwrap();

        assert_eq!(written, read);
        assert_eq!(request, read_request);
    }
}
