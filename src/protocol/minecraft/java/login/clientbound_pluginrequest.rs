use crate::network::packet::{Packet, PacketCodec};
use crate::protocol::types::{ByteArray, Identifier, ProtocolRead, VarInt};
use std::convert::TryFrom;

pub const CLIENTBOUND_PLUGIN_REQUEST_ID: i32 = 0x04;

#[derive(Debug, Clone)]
pub struct ClientBoundPluginRequest {
    pub message_id: VarInt,
    pub channel: Identifier,
    pub data: ByteArray, // Changé de Vec<u8> à ByteArray
}

impl TryFrom<&Packet> for ClientBoundPluginRequest {
    type Error = std::io::Error;

    fn try_from(packet: &Packet) -> Result<Self, Self::Error> {
        use std::io::Cursor;
        let mut cursor = Cursor::new(&packet.data);

        let (message_id, _) = VarInt::read_from(&mut cursor)?;
        let (channel, _) = Identifier::read_from(&mut cursor)?;
        let (data, _) = ByteArray::read_from(&mut cursor)?;

        Ok(Self {
            message_id,
            channel,
            data,
        })
    }
}

impl From<&ClientBoundPluginRequest> for Packet {
    fn from(request: &ClientBoundPluginRequest) -> Self {
        let mut packet = Packet::new(CLIENTBOUND_PLUGIN_REQUEST_ID);
        packet.encode(&request.message_id).unwrap();
        packet.encode(&request.channel).unwrap();
        packet.encode(&request.data).unwrap();
        packet
    }
}
