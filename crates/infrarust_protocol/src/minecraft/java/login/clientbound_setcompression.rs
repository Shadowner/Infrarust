use crate::network::packet::{Packet, PacketCodec};
use crate::protocol::types::{ProtocolRead, VarInt};
use std::convert::TryFrom;

pub const CLIENTBOUND_SET_COMPRESSION_ID: i32 = 0x03;
pub const DEFAULT_COMPRESSION_THRESHOLD: i32 = 256; // Seuil par défaut de compression

#[derive(Debug, Clone)]
pub struct ClientBoundSetCompression {
    pub threshold: VarInt,
}

impl TryFrom<&Packet> for ClientBoundSetCompression {
    type Error = std::io::Error;

    fn try_from(packet: &Packet) -> Result<Self, Self::Error> {
        use std::io::Cursor;
        let mut cursor = Cursor::new(&packet.data);
        let (threshold, _) = VarInt::read_from(&mut cursor)?;

        Ok(Self { threshold })
    }
}

impl From<&ClientBoundSetCompression> for Packet {
    fn from(compression: &ClientBoundSetCompression) -> Self {
        let mut packet = Packet::new(CLIENTBOUND_SET_COMPRESSION_ID);
        packet.encode(&compression.threshold).unwrap();
        packet
    }
}
