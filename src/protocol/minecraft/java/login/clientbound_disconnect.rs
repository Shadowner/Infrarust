use crate::network::packet::{Packet, PacketCodec};
use crate::protocol::types::{Chat, ProtocolRead, ProtocolString, ProtocolWrite};
use std::convert::TryFrom;
use std::io;

pub const CLIENTBOUND_DISCONNECT_ID: i32 = 0x00;

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

impl TryFrom<&Packet> for ClientBoundDisconnect {
    type Error = io::Error;

    fn try_from(packet: &Packet) -> Result<Self, Self::Error> {
        use std::io::Cursor;
        let mut cursor = Cursor::new(&packet.data);
        let (reason, _) = ProtocolString::read_from(&mut cursor)?;
        Ok(Self { reason })
    }
}

impl From<&ClientBoundDisconnect> for Packet {
    fn from(disconnect: &ClientBoundDisconnect) -> Self {
        let mut packet = Packet::new(0x00);
        packet.encode(&disconnect.reason).unwrap();
        packet
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
        let disconnect = ClientBoundDisconnect::new("Server closed".to_string());
        let mut buffer = Vec::new();
        let written = disconnect.write_to(&mut buffer).unwrap();
        assert!(written > 0);
    }
}
