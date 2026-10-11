#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use infrarust_api::handoff::{Handoff, HandoffTarget};
use infrarust_api::types::ServerId;
use infrarust_config::{DomainRewrite, KeepaliveConfig, ServerConfig};
use infrarust_protocol::Packet;
use infrarust_protocol::io::PacketDecoder;
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::version::ProtocolVersion;
use infrarust_transport::{BackendConnector, ForwardEndReason, ForwardResult};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{HandoffError, HandoffLevel, HandoffRelay};
use crate::limbo::test_helpers::test_proxy_services;
use crate::middleware::backend_selection::BackendTargets;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{ConnectionIntent, HandshakeData, RoutingData};
use crate::provider::ProviderId;
use crate::services::ProxyServices;
use crate::session::test_peer::raw_handshake;

const WAIT: Duration = Duration::from_secs(10);

const PP_V2_SIGNATURE: &[u8] = b"\r\n\r\n\x00\r\nQUIT\n";

struct Client {
    peer: TcpStream,
    ctx: ConnectionContext,
}

async fn client() -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local = listener.local_addr().unwrap();
    let peer = TcpStream::connect(local).await.unwrap();
    let (stream, remote) = listener.accept().await.unwrap();
    let ctx = ConnectionContext::new_for_test(stream, remote, remote.ip(), local);
    Client { peer, ctx }
}

async fn listener() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    (listener, addr)
}

fn relay(services: ProxyServices) -> HandoffRelay {
    HandoffRelay::new(
        services,
        Arc::new(BackendConnector::new(
            Duration::from_secs(2),
            KeepaliveConfig::default(),
        )),
    )
}

fn server(name: &str, backend: SocketAddr, edit: impl FnOnce(&mut ServerConfig)) -> ServerConfig {
    let mut config: ServerConfig = toml::from_str(&format!(
        "name = \"{name}\"\ndomains = [\"{name}.test\"]\naddresses = [\"{backend}\"]\n"
    ))
    .unwrap();
    edit(&mut config);
    config
}

fn add(services: &ProxyServices, config: ServerConfig) {
    let id = ProviderId::new("file", format!("{}.toml", config.effective_id()));
    services.domain_router.add(id, config);
}

fn spawn(
    relay: HandoffRelay,
    ctx: ConnectionContext,
    handoff: Handoff,
    level: HandoffLevel,
    shutdown: CancellationToken,
) -> JoinHandle<Result<ForwardResult, HandoffError>> {
    tokio::spawn(async move { relay.relay(ctx, handoff, level, shutdown).await })
}

async fn accept(listener: &TcpListener) -> TcpStream {
    tokio::time::timeout(WAIT, listener.accept())
        .await
        .expect("the relay never connected")
        .unwrap()
        .0
}

async fn read_exact(stream: &mut TcpStream, len: usize) -> Vec<u8> {
    let mut buf = vec![0; len];
    tokio::time::timeout(WAIT, stream.read_exact(&mut buf))
        .await
        .expect("the bytes never came")
        .unwrap();
    buf
}

async fn assert_nothing_more(stream: &mut TcpStream) {
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_millis(150), stream.read(&mut buf)).await;
    assert!(read.is_err(), "unexpected bytes: {read:?}");
}

async fn hang_up(peer: TcpStream, mut backend: TcpStream) {
    drop(peer);
    let mut rest = Vec::new();
    tokio::time::timeout(WAIT, backend.read_to_end(&mut rest))
        .await
        .expect("the client close never reached the target")
        .unwrap();
    assert!(rest.is_empty(), "unexpected bytes: {rest:?}");
}

async fn finish(task: JoinHandle<Result<ForwardResult, HandoffError>>) -> ForwardResult {
    tokio::time::timeout(WAIT, task)
        .await
        .expect("the relay never ended")
        .unwrap()
        .expect("the relay failed")
}

async fn ping_pong(peer: &mut TcpStream, backend: &mut TcpStream) {
    peer.write_all(b"ping").await.unwrap();
    assert_eq!(read_exact(backend, 4).await, b"ping");
    backend.write_all(b"pong").await.unwrap();
    assert_eq!(read_exact(peer, 4).await, b"pong");
}

fn handshake_data(host: &str, version: ProtocolVersion) -> HandshakeData {
    HandshakeData {
        domain: "origin.test".into(),
        raw_host: host.into(),
        port: 25565,
        protocol_version: version,
        intent: ConnectionIntent::Login,
        raw_packets: vec![raw_handshake(version, host)],
    }
}

fn routed(services: &ProxyServices, ctx: &mut ConnectionContext, server: &str) {
    let (server_config, load_balancer) = services
        .domain_router
        .find_route_by_server_id(server)
        .unwrap();
    ctx.extensions.insert(RoutingData {
        server_config,
        config_id: server.into(),
        load_balancer,
    });
}

#[tokio::test]
async fn raw_bytes_are_replayed_after_the_prefix_and_the_reply_reaches_the_client() {
    let (backend, addr) = listener().await;
    let Client { mut peer, mut ctx } = client().await;
    ctx.buffered_data = BytesMut::from(&b"AMMH\x00\x01hello"[..]);
    let task = spawn(
        relay(test_proxy_services()),
        ctx,
        Handoff::to(HandoffTarget::address("127.0.0.1", addr.port()))
            .with_prefix(&b"PRE"[..])
            .with_reply(&b"REPLY"[..]),
        HandoffLevel::Raw,
        CancellationToken::new(),
    );

    let mut backend = accept(&backend).await;
    assert_eq!(read_exact(&mut backend, 14).await, b"PREAMMH\x00\x01hello");
    assert_eq!(read_exact(&mut peer, 5).await, b"REPLY");
    ping_pong(&mut peer, &mut backend).await;

    hang_up(peer, backend).await;
    let result = finish(task).await;
    assert_eq!(result.client_to_backend, 4);
    assert_eq!(result.backend_to_client, 4);
    assert!(matches!(result.reason, ForwardEndReason::ClientClosed));
}

#[tokio::test]
async fn without_replay_only_the_prefix_and_the_live_bytes_reach_the_target() {
    let (backend, addr) = listener().await;
    let Client { mut peer, mut ctx } = client().await;
    ctx.buffered_data = BytesMut::from(&b"AMMH-peeked"[..]);
    let task = spawn(
        relay(test_proxy_services()),
        ctx,
        Handoff::to(HandoffTarget::address("127.0.0.1", addr.port()))
            .with_prefix(&b"HELLO"[..])
            .without_replay(),
        HandoffLevel::Raw,
        CancellationToken::new(),
    );

    let mut backend = accept(&backend).await;
    assert_eq!(read_exact(&mut backend, 5).await, b"HELLO");
    assert_nothing_more(&mut backend).await;
    ping_pong(&mut peer, &mut backend).await;
    drop(backend);
    let mut rest = Vec::new();
    tokio::time::timeout(WAIT, peer.read_to_end(&mut rest))
        .await
        .expect("the target close never reached the client")
        .unwrap();
    assert!(rest.is_empty());
    drop(peer);
    let result = finish(task).await;
    assert!(matches!(result.reason, ForwardEndReason::BackendClosed));
}

#[tokio::test]
async fn an_address_target_sends_a_proxy_header_only_when_asked() {
    for (asked, expected) in [(Some(true), true), (None, false), (Some(false), false)] {
        let (backend, addr) = listener().await;
        let Client { mut peer, mut ctx } = client().await;
        let client_addr = ctx.client_addr();
        ctx.buffered_data = BytesMut::from(&b"AMMH"[..]);
        let mut handoff = Handoff::to(HandoffTarget::address("127.0.0.1", addr.port()));
        if let Some(asked) = asked {
            handoff = handoff.with_proxy_protocol(asked);
        }
        let task = spawn(
            relay(test_proxy_services()),
            ctx,
            handoff,
            HandoffLevel::Raw,
            CancellationToken::new(),
        );
        let mut backend = accept(&backend).await;
        if expected {
            let header = read_exact(&mut backend, 16).await;
            assert_eq!(&header[..12], PP_V2_SIGNATURE, "{asked:?}");
            let len = usize::from(u16::from_be_bytes([header[14], header[15]]));
            let body = read_exact(&mut backend, len).await;
            assert_eq!(header[13], 0x11, "IPv4 over TCP");
            assert_eq!(&body[..4], &[127, 0, 0, 1]);
            assert_eq!(
                u16::from_be_bytes([body[8], body[9]]),
                client_addr.port(),
                "the header names the client"
            );
        }
        assert_eq!(read_exact(&mut backend, 4).await, b"AMMH", "{asked:?}");
        ping_pong(&mut peer, &mut backend).await;
        hang_up(peer, backend).await;
        finish(task).await;
    }
}

#[tokio::test]
async fn a_server_target_follows_its_proxy_protocol_setting_unless_overridden() {
    for (asked, expected) in [(None, true), (Some(false), false)] {
        let services = test_proxy_services();
        let (backend, addr) = listener().await;
        add(
            &services,
            server("modded", addr, |config| config.send_proxy_protocol = true),
        );
        let Client { mut peer, mut ctx } = client().await;
        ctx.buffered_data = BytesMut::from(&b"AMMH"[..]);
        let mut handoff = Handoff::to(HandoffTarget::server("modded"));
        if let Some(asked) = asked {
            handoff = handoff.with_proxy_protocol(asked);
        }
        let task = spawn(
            relay(services),
            ctx,
            handoff,
            HandoffLevel::Raw,
            CancellationToken::new(),
        );
        let mut backend = accept(&backend).await;
        let first = read_exact(&mut backend, 4).await;
        if expected {
            assert_eq!(&first[..], &PP_V2_SIGNATURE[..4]);
            let rest = read_exact(&mut backend, 12).await;
            let len = usize::from(u16::from_be_bytes([rest[10], rest[11]]));
            read_exact(&mut backend, len).await;
            assert_eq!(read_exact(&mut backend, 4).await, b"AMMH");
        } else {
            assert_eq!(first, b"AMMH");
        }
        ping_pong(&mut peer, &mut backend).await;
        hang_up(peer, backend).await;
        finish(task).await;
    }
}

#[tokio::test]
async fn a_server_port_target_reaches_the_picked_host_on_the_other_port() {
    let services = test_proxy_services();
    let (_minecraft, minecraft_addr) = listener().await;
    let (files, files_addr) = listener().await;
    add(&services, server("modded", minecraft_addr, |_| {}));
    let Client { mut peer, mut ctx } = client().await;
    ctx.buffered_data = BytesMut::from(&b"AMMH"[..]);
    let task = spawn(
        relay(services),
        ctx,
        Handoff::to(HandoffTarget::server_port("modded", files_addr.port())),
        HandoffLevel::Raw,
        CancellationToken::new(),
    );
    let mut backend = accept(&files).await;
    assert_eq!(read_exact(&mut backend, 4).await, b"AMMH");
    ping_pong(&mut peer, &mut backend).await;
    hang_up(peer, backend).await;
    finish(task).await;
}

#[tokio::test]
async fn an_unknown_server_is_refused_before_anything_is_written() {
    let Client { mut peer, ctx } = client().await;
    let refused = relay(test_proxy_services())
        .relay(
            ctx,
            Handoff::to(HandoffTarget::server("nowhere")).with_reply(&b"REPLY"[..]),
            HandoffLevel::Raw,
            CancellationToken::new(),
        )
        .await;
    assert!(
        matches!(&refused, Err(HandoffError::UnknownServer(server)) if *server == ServerId::new("nowhere")),
        "{refused:?}"
    );
    let mut rest = Vec::new();
    peer.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty(), "the reply must not be sent: {rest:?}");

    let Client { ctx, .. } = client().await;
    let refused = relay(test_proxy_services())
        .relay(
            ctx,
            Handoff::to(HandoffTarget::server_port("nowhere", 1)),
            HandoffLevel::Raw,
            CancellationToken::new(),
        )
        .await;
    assert!(matches!(refused, Err(HandoffError::UnknownServer(_))));
}

#[tokio::test]
async fn a_route_target_needs_a_routed_connection() {
    let Client { ctx, .. } = client().await;
    let refused = relay(test_proxy_services())
        .relay(
            ctx,
            Handoff::route(),
            HandoffLevel::Raw,
            CancellationToken::new(),
        )
        .await;
    assert!(matches!(refused, Err(HandoffError::NoRoute)), "{refused:?}");
}

#[tokio::test]
async fn an_unreachable_target_is_a_connect_error() {
    let (closed, addr) = listener().await;
    drop(closed);
    let Client { ctx, .. } = client().await;
    let refused = relay(test_proxy_services())
        .relay(
            ctx,
            Handoff::to(HandoffTarget::address("127.0.0.1", addr.port())),
            HandoffLevel::Raw,
            CancellationToken::new(),
        )
        .await;
    assert!(
        matches!(refused, Err(HandoffError::Connect(_))),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_handshake_handoff_replays_the_handshake_as_passthrough_forwards_it() {
    let services = test_proxy_services();
    let (backend, addr) = listener().await;
    add(&services, server("origin", addr, |_| {}));
    let Client { mut peer, mut ctx } = client().await;
    routed(&services, &mut ctx, "origin");
    let handshake = handshake_data("origin.test\0FML3\0", ProtocolVersion::V1_21);
    let raw = handshake.raw_packets[0].clone();
    ctx.extensions.insert(handshake);
    ctx.buffered_data = BytesMut::from(&b"\x01\x00"[..]);
    let task = spawn(
        relay(services),
        ctx,
        Handoff::route(),
        HandoffLevel::Handshake,
        CancellationToken::new(),
    );
    let mut backend = accept(&backend).await;
    assert_eq!(read_exact(&mut backend, raw.len()).await, raw);
    assert_eq!(read_exact(&mut backend, 2).await, b"\x01\x00");
    ping_pong(&mut peer, &mut backend).await;
    hang_up(peer, backend).await;
    finish(task).await;
}

#[tokio::test]
async fn a_login_handoff_replays_the_rewritten_handshake_then_the_login_start_as_received() {
    let services = test_proxy_services();
    let (_origin, origin_addr) = listener().await;
    let (backend, addr) = listener().await;
    add(&services, server("origin", origin_addr, |_| {}));
    add(
        &services,
        server("modded", addr, |config| {
            config.domain_rewrite = DomainRewrite::Explicit("backend.internal".into());
        }),
    );
    let Client { mut peer, mut ctx } = client().await;
    routed(&services, &mut ctx, "origin");
    let mut handshake = handshake_data("origin.test\0FML3\0", ProtocolVersion::V1_21);
    let login_start = BytesMut::from(&b"\x07\x00\x05Steve"[..]);
    handshake.raw_packets.push(login_start.clone());
    ctx.extensions.insert(handshake);
    ctx.buffered_data = BytesMut::from(&b"\x02\x02\x00"[..]);
    let task = spawn(
        relay(services),
        ctx,
        Handoff::to(HandoffTarget::server("modded")).with_prefix(&b"P"[..]),
        HandoffLevel::Login,
        CancellationToken::new(),
    );
    let mut backend = accept(&backend).await;
    assert_eq!(read_exact(&mut backend, 1).await, b"P");
    let mut decoder = PacketDecoder::new();
    let frame = loop {
        if let Some(frame) = decoder.try_next_frame().unwrap() {
            break frame;
        }
        decoder.queue_bytes(&read_exact(&mut backend, 1).await);
    };
    let replayed =
        SHandshake::decode(&mut frame.payload.as_ref(), ProtocolVersion::V1_7_2).unwrap();
    assert_eq!(replayed.server_address, "backend.internal\0FML3\0");
    assert!(decoder.into_remaining().is_empty());
    assert_eq!(
        read_exact(&mut backend, login_start.len()).await,
        login_start
    );
    assert_eq!(read_exact(&mut backend, 3).await, b"\x02\x02\x00");
    ping_pong(&mut peer, &mut backend).await;
    hang_up(peer, backend).await;
    finish(task).await;
}

#[tokio::test]
async fn a_route_handoff_uses_the_addresses_already_picked_for_the_connection() {
    let services = test_proxy_services();
    let (_configured, configured_addr) = listener().await;
    let (picked, picked_addr) = listener().await;
    add(&services, server("origin", configured_addr, |_| {}));
    let Client { mut peer, mut ctx } = client().await;
    routed(&services, &mut ctx, "origin");
    ctx.extensions.insert(BackendTargets {
        addresses: smallvec::smallvec![infrarust_config::ServerAddress {
            host: "127.0.0.1".into(),
            port: picked_addr.port(),
        }],
    });
    ctx.buffered_data = BytesMut::from(&b"AMMH"[..]);
    let task = spawn(
        relay(services),
        ctx,
        Handoff::route(),
        HandoffLevel::Raw,
        CancellationToken::new(),
    );
    let mut backend = accept(&picked).await;
    assert_eq!(read_exact(&mut backend, 4).await, b"AMMH");
    ping_pong(&mut peer, &mut backend).await;
    hang_up(peer, backend).await;
    finish(task).await;
}

#[tokio::test]
async fn the_proxy_shutdown_ends_a_relay() {
    let (backend, addr) = listener().await;
    let Client { mut peer, ctx } = client().await;
    let shutdown = CancellationToken::new();
    let task = spawn(
        relay(test_proxy_services()),
        ctx,
        Handoff::to(HandoffTarget::address("127.0.0.1", addr.port())),
        HandoffLevel::Raw,
        shutdown.clone(),
    );
    let mut backend = accept(&backend).await;
    ping_pong(&mut peer, &mut backend).await;
    shutdown.cancel();
    let result = finish(task).await;
    assert!(matches!(result.reason, ForwardEndReason::Shutdown));
}

struct Pattern(u64);

impl Pattern {
    fn fill(&mut self, buf: &mut [u8]) {
        for byte in buf {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            *byte = self.0.to_le_bytes()[0];
        }
    }
}

const LARGE: usize = 64 * 1024 * 1024;

async fn send_pattern(mut writer: tokio::net::tcp::OwnedWriteHalf, seed: u64) {
    let mut pattern = Pattern(seed);
    let mut chunk = vec![0u8; 256 * 1024];
    let mut sent = 0;
    while sent < LARGE {
        let len = chunk.len().min(LARGE - sent);
        pattern.fill(&mut chunk[..len]);
        writer.write_all(&chunk[..len]).await.unwrap();
        sent += len;
    }
    writer.shutdown().await.unwrap();
}

async fn check_pattern(mut reader: tokio::net::tcp::OwnedReadHalf, seed: u64) -> usize {
    let mut pattern = Pattern(seed);
    let mut buf = vec![0u8; 256 * 1024];
    let mut expected = vec![0u8; 256 * 1024];
    let mut received = 0;
    loop {
        let read = reader.read(&mut buf).await.unwrap();
        if read == 0 {
            return received;
        }
        pattern.fill(&mut expected[..read]);
        assert_eq!(
            buf[..read],
            expected[..read],
            "corrupted at byte {received}"
        );
        received += read;
    }
}

async fn large_transfer(zero_copy: bool) {
    let (backend, addr) = listener().await;
    let Client { peer, ctx } = client().await;
    let task = spawn(
        relay(test_proxy_services()).with_zero_copy(zero_copy),
        ctx,
        Handoff::to(HandoffTarget::address("127.0.0.1", addr.port())).without_replay(),
        HandoffLevel::Raw,
        CancellationToken::new(),
    );
    let backend = accept(&backend).await;
    let (peer_read, peer_write) = peer.into_split();
    let (backend_read, backend_write) = backend.into_split();
    let up = tokio::spawn(send_pattern(peer_write, 0x9E37_79B9_7F4A_7C15));
    let down = tokio::spawn(send_pattern(backend_write, 0xD1B5_4A32_D192_ED03));
    let at_backend = tokio::spawn(check_pattern(backend_read, 0x9E37_79B9_7F4A_7C15));
    let at_client = tokio::spawn(check_pattern(peer_read, 0xD1B5_4A32_D192_ED03));
    let deadline = Duration::from_secs(120);
    tokio::time::timeout(deadline, up).await.unwrap().unwrap();
    tokio::time::timeout(deadline, down).await.unwrap().unwrap();
    assert_eq!(
        tokio::time::timeout(deadline, at_backend)
            .await
            .unwrap()
            .unwrap(),
        LARGE
    );
    assert_eq!(
        tokio::time::timeout(deadline, at_client)
            .await
            .unwrap()
            .unwrap(),
        LARGE
    );
    let result = tokio::time::timeout(deadline, task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.client_to_backend, LARGE as u64);
    assert_eq!(result.backend_to_client, LARGE as u64);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sixty_four_mebibytes_cross_each_way_unchanged_through_the_zero_copy_relay() {
    large_transfer(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sixty_four_mebibytes_cross_each_way_unchanged_through_the_copy_relay() {
    large_transfer(false).await;
}
