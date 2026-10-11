#![cfg(test)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use infrarust_config::ServerConfig;
use infrarust_protocol::Packet;
use infrarust_protocol::io::{PacketDecoder, PacketEncoder, PacketFrame};
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::login::{
    CLoginPluginRequest, CLoginSuccess, CSetCompression, SLoginAcknowledged, SLoginPluginResponse,
    SLoginStart,
};
use infrarust_protocol::registry::{PacketRegistry, build_default_registry};
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use infrarust_protocol::{MAX_PACKET_SIZE, VarInt};

use crate::forwarding::ForwardingMode;
use crate::limbo::test_helpers::{build_frame, test_proxy_services};
use crate::services::ProxyServices;
use crate::session::client_bridge::ClientBridge;
use crate::session::context::SessionContext;

pub(crate) const V: ProtocolVersion = ProtocolVersion::V1_21;

const PEER_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct Peer {
    stream: TcpStream,
    decoder: PacketDecoder,
    encoder: PacketEncoder,
    version: ProtocolVersion,
    registry: PacketRegistry,
}

impl Peer {
    pub(crate) fn new(stream: TcpStream, version: ProtocolVersion) -> Self {
        Self {
            stream,
            decoder: PacketDecoder::new(),
            encoder: PacketEncoder::new(),
            version,
            registry: build_default_registry(),
        }
    }

    pub(crate) async fn accept(listener: &TcpListener, version: ProtocolVersion) -> Self {
        let (stream, _) = tokio::time::timeout(PEER_TIMEOUT, listener.accept())
            .await
            .expect("nobody connected")
            .unwrap();
        Self::new(stream, version)
    }

    pub(crate) fn set_compression(&mut self, threshold: i32) {
        self.decoder.set_compression(threshold);
        self.encoder.set_compression(threshold);
    }

    pub(crate) fn id<P: Packet>(&self) -> i32 {
        self.registry
            .get_packet_id::<P>(self.version)
            .expect(P::NAME)
    }

    pub(crate) async fn recv(&mut self) -> Option<PacketFrame> {
        tokio::time::timeout(PEER_TIMEOUT, self.next())
            .await
            .expect("the peer waited too long for a packet")
    }

    pub(crate) async fn next(&mut self) -> Option<PacketFrame> {
        loop {
            if let Some(frame) = self.decoder.try_next_frame().unwrap() {
                return Some(frame);
            }
            let buf = self.decoder.read_buf_mut();
            buf.reserve(16 * 1024);
            if self.stream.read_buf(buf).await.unwrap_or(0) == 0 {
                return None;
            }
        }
    }

    pub(crate) async fn frame(&mut self) -> PacketFrame {
        self.recv().await.expect("the connection closed")
    }

    pub(crate) async fn expect<P: Packet>(&mut self) -> P {
        let frame = self.frame().await;
        assert_eq!(frame.id, self.id::<P>(), "expected {}", P::NAME);
        P::decode(&mut frame.payload.as_ref(), self.version).unwrap()
    }

    pub(crate) async fn send_frame(&mut self, frame: &PacketFrame) {
        self.encoder.append_raw(frame.id, &frame.payload).unwrap();
        self.stream.write_all(&self.encoder.take()).await.unwrap();
    }

    pub(crate) async fn send<P: Packet>(&mut self, packet: &P) -> PacketFrame {
        let frame = build_frame(packet, self.version, &self.registry);
        self.send_frame(&frame).await;
        frame
    }
}

pub(crate) async fn client_pair(version: ProtocolVersion) -> (ClientBridge, Peer) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let peer = TcpStream::connect(addr).await.unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let mut client = ClientBridge::new(stream, BytesMut::new(), version);
    client.set_state(ConnectionState::Login);
    (client, Peer::new(peer, version))
}

pub(crate) fn raw_handshake(version: ProtocolVersion, host: &str) -> BytesMut {
    let handshake = SHandshake {
        protocol_version: VarInt(version.0),
        server_address: host.to_string(),
        server_port: 25565,
        next_state: ConnectionState::Login,
        transfer: false,
    };
    let mut payload = Vec::new();
    handshake.encode(&mut payload, version).unwrap();
    let mut encoder = PacketEncoder::new();
    encoder.append_raw(0x00, &payload).unwrap();
    encoder.take()
}

pub(crate) fn with_handshake(mut ctx: SessionContext<'_>) -> SessionContext<'_> {
    ctx.handshake.raw_packets = vec![raw_handshake(V, "origin.test")];
    ctx
}

pub(crate) fn with_login_start(mut ctx: SessionContext<'_>) -> SessionContext<'_> {
    let start = SLoginStart {
        name: "TestPlayer".to_string(),
        uuid: Some(uuid::Uuid::nil()),
        profile_key: None,
    };
    let frame = build_frame(&start, V, &build_default_registry());
    let mut encoder = PacketEncoder::new();
    encoder.append_raw(frame.id, &frame.payload).unwrap();
    ctx.handshake.raw_packets = vec![raw_handshake(V, "origin.test"), encoder.take()];
    ctx
}

pub(crate) fn velocity_services() -> ProxyServices {
    let mut services = test_proxy_services();
    services.forwarding_mode = Arc::new(ForwardingMode::Velocity {
        secret: b"secret".to_vec(),
    });
    services.forwarding_secret = Some(Arc::from(&b"secret"[..]));
    services
}

pub(crate) fn server_config(backend: &TcpListener, extra: &str) -> ServerConfig {
    let address = backend.local_addr().unwrap();
    toml::from_str(&format!(
        "name = \"origin\"\ndomains = [\"origin.test\"]\naddresses = [\"{address}\"]\nproxy_mode = \"offline\"\n{extra}"
    ))
    .unwrap()
}

pub(crate) fn plugin_request(message_id: i32, channel: &str, data: &[u8]) -> CLoginPluginRequest {
    CLoginPluginRequest {
        message_id: VarInt(message_id),
        channel: channel.to_string(),
        data: data.to_vec(),
    }
}

pub(crate) fn plugin_response(message_id: i32, data: &[u8]) -> SLoginPluginResponse {
    SLoginPluginResponse {
        message_id: VarInt(message_id),
        successful: true,
        data: data.to_vec(),
    }
}

pub(crate) fn backend_success() -> CLoginSuccess {
    CLoginSuccess {
        uuid: uuid::Uuid::nil(),
        username: "TestPlayer".to_string(),
        properties: vec![],
        strict_error_handling: false,
        session_id: None,
    }
}

pub(crate) async fn backend_login(listener: &TcpListener) -> (Peer, SHandshake) {
    let mut conn = Peer::accept(listener, V).await;
    let handshake = conn.expect::<SHandshake>().await;
    conn.expect::<SLoginStart>().await;
    (conn, handshake)
}

pub(crate) async fn finish_backend_login(conn: &mut Peer) {
    conn.send(&backend_success()).await;
    conn.expect::<SLoginAcknowledged>().await;
}

pub(crate) async fn finish_client_login(peer: &mut Peer) -> i32 {
    let compression = peer.expect::<CSetCompression>().await;
    peer.set_compression(compression.threshold.0);
    peer.expect::<CLoginSuccess>().await;
    peer.send(&SLoginAcknowledged).await;
    compression.threshold.0
}

pub(crate) fn var_int(value: i32) -> Vec<u8> {
    let mut buf = Vec::new();
    VarInt(value).encode(&mut buf).unwrap();
    buf
}

pub(crate) fn oversized_payload() -> Bytes {
    Bytes::from(vec![7u8; MAX_PACKET_SIZE + 1024 * 1024])
}
