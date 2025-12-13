use infrarust_protocol::minecraft::java::handshake::ServerBoundHandshake;

pub struct HandshakeEvent {
    pub packet: ServerBoundHandshake,
}
