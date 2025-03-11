use crate::protocol::types::ProtocolWrite;
use std::io;

pub const SERVERBOUND_REQUEST_ID: i32 = 0x00;

#[derive(Debug, Clone, PartialEq)]
pub struct ServerBoundRequest;

impl Default for ServerBoundRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerBoundRequest {
    pub fn new() -> Self {
        ServerBoundRequest
    }
}

impl ProtocolWrite for ServerBoundRequest {
    fn write_to<W: io::Write>(&self, _writer: &mut W) -> io::Result<usize> {
        // Empty packet - no data to write
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_write() {
        let request = ServerBoundRequest::new();
        let mut buffer = Vec::new();
        let written = request.write_to(&mut buffer).unwrap();
        assert_eq!(written, 0);
        assert!(buffer.is_empty());
    }
}
