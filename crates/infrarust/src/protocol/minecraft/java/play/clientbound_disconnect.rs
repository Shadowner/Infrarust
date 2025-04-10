use crate::protocol::types::{Chat, ProtocolString, ProtocolWrite};
use std::io;

pub const CLIENTBOUND_DISCONNECT_ID: i32 = 0x17;

#[derive(Debug, Clone, PartialEq)]
pub struct ClientBoundDisconnect {
    pub reason: Chat,
}

impl ClientBoundDisconnect {
    pub fn new(reason: String) -> Self {
        Self {
            reason: ProtocolString(reason),
        }
    }
}

impl ProtocolWrite for ClientBoundDisconnect {
    fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        self.reason.write_to(writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disconnect_write() {
        let disconnect = ClientBoundDisconnect::new("Server disconnected".to_string());
        let mut buffer = Vec::new();
        let written = disconnect.write_to(&mut buffer).unwrap();
        assert!(written > 0);
    }
}
