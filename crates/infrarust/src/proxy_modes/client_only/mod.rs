pub mod client;
pub mod server;

use infrarust_protocol::{
    ProtocolRead, ProtocolWrite,
    minecraft::java::handshake::{SERVERBOUND_HANDSHAKE_ID, ServerBoundHandshake},
    types::{Byte, ProtocolString, UnsignedShort, VarInt},
};

use crate::network::packet::Packet;
use std::io::{self};

use super::{ProxyMessage, ProxyModeMessageType};

pub struct ClientOnlyMode;

#[derive(Debug)]
pub enum ClientOnlyMessage {
    ClientReady(),
    ServerReady(),
    ClientLoginAcknowledged(Packet),

    ServerThreshold(VarInt),
}

impl ProxyMessage for ClientOnlyMessage {}

impl ProxyModeMessageType for ClientOnlyMode {
    type Message = ClientOnlyMessage;
}

fn prepare_server_handshake(
    client_handshake: &Packet,
    server_addr: &std::net::SocketAddr,
) -> io::Result<Packet> {
    let mut cursor = std::io::Cursor::new(&client_handshake.data);
    let (protocol_version, _) = VarInt::read_from(&mut cursor)?;

    let server_handshale = ServerBoundHandshake {
        protocol_version,
        server_address: ProtocolString(server_addr.ip().to_string()),
        server_port: UnsignedShort(server_addr.port()),
        next_state: Byte(2),
    };

    let handshake = Packet::try_from(&server_handshale).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to create server handshake packet: {}", e),
        )
    })?;
    Ok(handshake)
}

impl TryFrom<&Packet> for ServerBoundHandshake {
    type Error = io::Error;

    fn try_from(packet: &Packet) -> Result<Self, Self::Error> {
        Self::from_packet(packet)
    }
}

impl TryFrom<&ServerBoundHandshake> for Packet {
    type Error = io::Error;

    fn try_from(handshake: &ServerBoundHandshake) -> Result<Self, Self::Error> {
        let mut handshake_packet = Packet::new(SERVERBOUND_HANDSHAKE_ID);
        let mut data = Vec::new();
        handshake.protocol_version.write_to(&mut data)?;
        handshake.server_address.write_to(&mut data)?;
        handshake.server_port.write_to(&mut data)?;
        VarInt(2).write_to(&mut data)?;
        handshake_packet.data = bytes::BytesMut::from(&data[..]);

        Ok(handshake_packet)
    }
}
