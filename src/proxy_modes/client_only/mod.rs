pub mod client;
pub mod server;

use crate::ProtocolRead;
use crate::network::packet::Packet;
use crate::protocol::minecraft::java::handshake::ServerBoundHandshake;
use crate::protocol::types::{Byte, ProtocolString, UnsignedShort, VarInt};
use std::io::{self};

use super::{ProxyMessage, ProxyModeMessageType};

pub struct ClientOnlyMode;

#[derive(Debug)]
pub enum ClientOnlyMessage {
    ClientReady(),
    ServerReady(),
    ClientLoginAknowledged(Packet),

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
