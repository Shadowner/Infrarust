use crate::network::packet::{Packet, PacketCodec}; // Ajout de PacketCodec
use crate::protocol::types::{Identifier, ProtocolRead};
use std::convert::TryFrom;

pub const CLIENTBOUND_COOKIE_REQUEST_ID: i32 = 0x05;

#[derive(Debug, Clone)]
pub struct ClientBoundCookieRequest {
    pub key: Identifier,
}

impl TryFrom<&Packet> for ClientBoundCookieRequest {
    type Error = std::io::Error;

    fn try_from(packet: &Packet) -> Result<Self, Self::Error> {
        use std::io::Cursor;
        let mut cursor = Cursor::new(&packet.data);
        let (key, _) = Identifier::read_from(&mut cursor)?;

        Ok(Self { key })
    }
}

impl From<&ClientBoundCookieRequest> for Packet {
    fn from(request: &ClientBoundCookieRequest) -> Self {
        let mut packet = Packet::new(CLIENTBOUND_COOKIE_REQUEST_ID);
        packet.encode(&request.key).unwrap();
        packet
    }
}
