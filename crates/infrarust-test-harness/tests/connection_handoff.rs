#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use infrarust_api::event::EventPriority;
use infrarust_api::events::claim::{LoginClaimEvent, RawConnectionEvent};
use infrarust_api::events::handshake::{ConnectionHandshakeEvent, HandshakeIntent};
use infrarust_api::handoff::{Handoff, HandoffTarget, validate_claim_prefix};
use infrarust_api::services::ban_service::{BanRequest, BanTarget};
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::login::SLoginStart;
use infrarust_protocol::{McBufWriteExt, Packet, PacketEncoder, VarInt};
use infrarust_test_harness::versions::CURRENT;
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, EventKind, FakeBackend, FakeClient, FramedConn,
    ProtocolVersion, Recorder, ScriptedPlugin, ServerSpec, TestProxy,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::time::timeout;
use uuid::Uuid;

const T: Duration = DEFAULT_TIMEOUT;
const BRIDGE: &str = "bridge";
const XFER: &[u8] = b"XFER";
const PIPELINED: &[u8] = b"\x16\x03\x01\x00\x05hello";
const MODERN_LAYOUT_VERSIONS: [i32; 2] = [763, 767];
const PARTIAL_MATCH_TIMEOUT: Duration = Duration::from_secs(5);
const MARKED_UUID: Uuid = Uuid::from_u128(0xa7c3_5e10_9b2d_4f6e_8a01_c4d2_e3f4_a5b6);

type Raw = Arc<Mutex<Vec<(Bytes, Bytes)>>>;

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut encoder = PacketEncoder::new();
    encoder.append_raw(0x00, payload).unwrap();
    encoder.take().to_vec()
}

fn handshake(domain: &str, version: i32, next_state: ConnectionState) -> Vec<u8> {
    handshake_to(domain, 25565, version, next_state)
}

fn handshake_to(domain: &str, port: u16, version: i32, next_state: ConnectionState) -> Vec<u8> {
    let packet = SHandshake {
        protocol_version: VarInt(version),
        server_address: domain.to_string(),
        server_port: port,
        next_state,
        transfer: false,
    };
    let mut payload = Vec::new();
    packet
        .encode(&mut payload, ProtocolVersion(version))
        .unwrap();
    frame(&payload)
}

fn modern_layout_login_start(name: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.write_string(name).unwrap();
    payload.write_uuid(&MARKED_UUID).unwrap();
    payload
}

fn declared_login_start(name: &str, version: i32) -> Vec<u8> {
    let packet = SLoginStart {
        name: name.to_string(),
        uuid: Some(MARKED_UUID),
        profile_key: None,
    };
    let mut payload = Vec::new();
    packet
        .encode(&mut payload, ProtocolVersion(version))
        .unwrap();
    payload
}

async fn listener() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    (listener, addr)
}

async fn accept(listener: &TcpListener) -> TcpStream {
    timeout(T, listener.accept()).await.unwrap().unwrap().0
}

async fn nothing_accepted(listener: &TcpListener) {
    let accepted = timeout(Duration::from_millis(300), listener.accept()).await;
    assert!(accepted.is_err(), "the target should not have been reached");
}

async fn read_n(stream: &mut TcpStream, n: usize) -> Vec<u8> {
    let mut received = vec![0; n];
    timeout(T, stream.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    received
}

async fn closed_silently(stream: &mut TcpStream) {
    let mut rest = Vec::new();
    let read = timeout(T, stream.read_to_end(&mut rest)).await.unwrap();
    assert!(read.is_err() || rest.is_empty(), "{rest:?}");
}

async fn closed(stream: &mut TcpStream) {
    let mut rest = Vec::new();
    let read = timeout(T, stream.read_to_end(&mut rest)).await;
    assert!(read.is_ok(), "the connection was never closed");
}

fn proxy_header(ip: &str, port: u16) -> Vec<u8> {
    format!("PROXY TCP4 {ip} 127.0.0.1 {port} 25565\r\n").into_bytes()
}

fn receive_proxy_protocol(table: &mut toml::Table) {
    table.insert("receive_proxy_protocol".into(), toml::Value::Boolean(true));
}

fn select_ban_provider(table: &mut toml::Table, provider: &str) {
    table
        .get_mut("ban")
        .and_then(toml::Value::as_table_mut)
        .expect("the harness writes a [ban] table")
        .insert("provider".into(), toml::Value::String(provider.into()));
}

async fn ban_ip(proxy: &TestProxy, ip: &str) {
    proxy
        .services()
        .ban_manager
        .issue(BanRequest::new(BanTarget::Ip(ip.parse().unwrap())).reason("griefing"))
        .await
        .unwrap();
}

async fn login_disconnect(client: TcpStream) -> String {
    let mut conn = FramedConn::new(client).unwrap();
    let frame = timeout(T, conn.read_frame())
        .await
        .expect("no login disconnect came")
        .unwrap()
        .expect("the connection closed without a login disconnect");
    assert_eq!(frame.id, 0x00);
    String::from_utf8_lossy(&frame.payload).into_owned()
}

async fn both_ways(client: &mut TcpStream, upstream: &mut TcpStream) {
    client.write_all(b"later").await.unwrap();
    assert_eq!(read_n(upstream, 5).await, b"later");
    upstream.write_all(b"pong").await.unwrap();
    assert_eq!(read_n(client, 4).await, b"pong");
}

async fn settle(proxy: &TestProxy) {
    timeout(T, proxy.bus().flush())
        .await
        .expect("the event queue never drained");
}

fn rejections(recorder: &Recorder) -> Vec<Value> {
    recorder
        .of(EventKind::ConnectionRejected)
        .into_iter()
        .map(|e| e.detail)
        .collect()
}

fn bridge(
    prefixes: Vec<Vec<u8>>,
    decide: impl Fn(&mut RawConnectionEvent) + Send + Sync + 'static,
) -> (ScriptedPlugin, Raw) {
    let seen: Raw = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    let plugin = ScriptedPlugin::new(BRIDGE)
        .on_enable(move |ctx| {
            let claims = ctx
                .connection_claims()
                .expect("the bridge holds connection-handoff");
            for prefix in &prefixes {
                claims.register_prefix(prefix).unwrap();
            }
        })
        .on::<RawConnectionEvent>(EventPriority::NORMAL, move |event| {
            record
                .lock()
                .unwrap()
                .push((event.prefix.clone(), event.data.clone()));
            decide(event);
        });
    (plugin, seen)
}

fn seen_count(seen: &Raw) -> usize {
    seen.lock().unwrap().len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_raw_claim_is_relayed_with_the_bytes_it_was_claimed_by() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (files, files_addr) = listener().await;
    let port = files_addr.port();
    let (plugin, seen) = bridge(vec![XFER.to_vec()], move |event| {
        event.handoff(
            Handoff::to(HandoffTarget::server_port("lobby", port))
                .with_prefix(&b"P:"[..])
                .with_reply(&b"R:"[..]),
        );
    });
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let opening = b"XFER\x00\x01hello";
    let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
    client.write_all(opening).await.unwrap();

    let mut upstream = accept(&files).await;
    assert_eq!(
        read_n(&mut upstream, 2 + opening.len()).await,
        [&b"P:"[..], opening].concat()
    );
    assert_eq!(read_n(&mut client, 2).await, b"R:");
    both_ways(&mut client, &mut upstream).await;
    drop(upstream);
    closed_silently(&mut client).await;

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, Bytes::from_static(XFER));
    assert!(seen[0].1.starts_with(XFER) && opening.starts_with(&seen[0].1));
    settle(&proxy).await;
    assert_eq!(recorder.count(EventKind::ConnectionHandshake), 0);
    assert_eq!(recorder.count(EventKind::PreLogin), 0);
    assert!(rejections(&recorder).is_empty());
    assert_eq!(backend.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_raw_claim_can_wait_for_more_bytes() {
    let (files, files_addr) = listener().await;
    let asked = Arc::new(Notify::new());
    let ask = Arc::clone(&asked);
    let (plugin, seen) = bridge(vec![XFER.to_vec()], move |event| {
        if event.data.len() < 12 {
            ask.notify_one();
            event.need_more(12);
        } else {
            event.handoff(Handoff::to(HandoffTarget::address(
                "127.0.0.1",
                files_addr.port(),
            )));
        }
    });
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(files_addr))
        .plugin(plugin)
        .start()
        .await
        .unwrap();

    let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
    client.write_all(XFER).await.unwrap();
    timeout(T, asked.notified()).await.unwrap();
    client.write_all(b"12345678rest").await.unwrap();

    let mut upstream = accept(&files).await;
    assert_eq!(read_n(&mut upstream, 16).await, b"XFER12345678rest");
    both_ways(&mut client, &mut upstream).await;

    let lengths: Vec<usize> = seen.lock().unwrap().iter().map(|(_, d)| d.len()).collect();
    assert_eq!(lengths, [4, 12]);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_partial_prefix_followed_by_another_byte_is_handled_as_minecraft() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let opening = handshake("lobby.test", CURRENT, ConnectionState::Status);
    let partial = vec![opening[0], b'Z'];
    validate_claim_prefix(&partial).unwrap();
    let (plugin, seen) = bridge(vec![partial], RawConnectionEvent::reject);
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .start()
        .await
        .unwrap();
    assert_eq!(proxy.domain("lobby"), Some("lobby.test"));

    let started = Instant::now();
    let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
    client.write_all(&opening[..1]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut rest = opening[1..].to_vec();
    rest.extend(frame(&[]));
    client.write_all(&rest).await.unwrap();

    let mut conn = FramedConn::new(client).unwrap();
    let response = timeout(T, conn.read_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.id, 0x00);
    assert!(started.elapsed() < PARTIAL_MATCH_TIMEOUT);
    assert_eq!(seen_count(&seen), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn minecraft_clients_wait_for_nothing_while_prefixes_are_registered() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let opening = handshake("lobby.test", CURRENT, ConnectionState::Status);
    let (plugin, seen) = bridge(
        vec![XFER.to_vec(), vec![opening[0], b'Z']],
        RawConnectionEvent::reject,
    );
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .start()
        .await
        .unwrap();

    let started = Instant::now();
    let status = proxy
        .client(ProtocolVersion(CURRENT))
        .status()
        .await
        .unwrap();
    assert!(status.json.get("description").is_some(), "{status:?}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a vanilla ping took {:?}",
        started.elapsed()
    );

    let session = proxy
        .client(ProtocolVersion(CURRENT))
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();
    assert_eq!(seen_count(&seen), 0);

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_registered_prefix_nothing_is_claimed() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (files, files_addr) = listener().await;
    let (plugin, seen) = bridge(Vec::new(), move |event| {
        event.handoff(Handoff::to(HandoffTarget::address(
            "127.0.0.1",
            files_addr.port(),
        )));
    });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .start()
        .await
        .unwrap();
    assert!(proxy.services().connection_claims.is_empty());

    let mut opening = XFER.to_vec();
    opening.extend([0; 128]);
    let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
    client.write_all(&opening).await.unwrap();
    closed_silently(&mut client).await;
    nothing_accepted(&files).await;

    let status = proxy
        .client(ProtocolVersion(CURRENT))
        .status()
        .await
        .unwrap();
    assert!(status.json.get("description").is_some(), "{status:?}");
    assert_eq!(seen_count(&seen), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn filtered_and_banned_peers_are_never_claimed_and_a_rejected_claim_is_closed() {
    let (files, files_addr) = listener().await;
    let (plugin, seen) = bridge(vec![XFER.to_vec()], RawConnectionEvent::reject);
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(files_addr))
        .plugin(plugin)
        .plugin(recorder.plugin())
        .patch_config(|table| {
            table.insert("receive_proxy_protocol".into(), toml::Value::Boolean(true));
            let ip_filter: toml::Table =
                toml::from_str("blacklist = [\"203.0.113.66/32\"]").unwrap();
            table.insert("ip_filter".into(), ip_filter.into());
        })
        .start()
        .await
        .unwrap();
    proxy
        .services()
        .ban_manager
        .issue(BanRequest::new(BanTarget::Ip(
            "203.0.113.7".parse().unwrap(),
        )))
        .await
        .unwrap();

    for (ip, port, claimed) in [
        ("203.0.113.66", 50001, 0),
        ("203.0.113.7", 50002, 0),
        ("198.51.100.1", 50003, 1),
    ] {
        let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
        client
            .write_all(&[proxy_header(ip, port).as_slice(), XFER, b"\x00\x01"].concat())
            .await
            .unwrap();
        if ip == "203.0.113.66" {
            closed(&mut client).await;
        } else {
            closed_silently(&mut client).await;
        }
        assert_eq!(seen_count(&seen), claimed, "{ip}");
    }
    nothing_accepted(&files).await;

    settle(&proxy).await;
    assert_eq!(
        rejections(&recorder),
        [
            json!({ "remote_addr": "203.0.113.66:50001", "virtual_host": null, "reason": "ip_filter", "plugin": null }),
            json!({ "remote_addr": "203.0.113.7:50002", "virtual_host": null, "reason": "ip_banned", "plugin": null }),
            json!({ "remote_addr": "198.51.100.1:50003", "virtual_host": null, "reason": "plugin", "plugin": BRIDGE }),
        ]
    );

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_filtered_peer_is_refused_at_once_while_prefixes_are_registered() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (plugin, seen) = bridge(vec![XFER.to_vec()], RawConnectionEvent::reject);
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .plugin(recorder.plugin())
        .patch_config(|table| {
            receive_proxy_protocol(table);
            let ip_filter: toml::Table =
                toml::from_str("blacklist = [\"203.0.113.66/32\"]").unwrap();
            table.insert("ip_filter".into(), ip_filter.into());
        })
        .start()
        .await
        .unwrap();

    let started = Instant::now();
    let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
    client
        .write_all(&[proxy_header("203.0.113.66", 50001).as_slice(), b"XF"].concat())
        .await
        .unwrap();
    closed(&mut client).await;
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a filtered peer held its slot for {:?}",
        started.elapsed()
    );
    assert_eq!(seen_count(&seen), 0);

    settle(&proxy).await;
    assert_eq!(
        rejections(&recorder),
        [
            json!({ "remote_addr": "203.0.113.66:50001", "virtual_host": null, "reason": "ip_filter", "plugin": null }),
        ]
    );

    proxy.shutdown().await.unwrap();
}

fn handshake_gate(intent: ConnectionState) -> ScriptedPlugin {
    ScriptedPlugin::new("gate").on::<ConnectionHandshakeEvent>(
        EventPriority::NORMAL,
        move |event| {
            let status = event.intent == HandshakeIntent::Status;
            if status == (intent == ConnectionState::Status) {
                event.handoff(Handoff::route());
            }
        },
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_login_claim_from_a_banned_address_is_refused_like_a_banned_login() {
    let (backend, backend_addr) = listener().await;
    let (plugin, claims) = login_bridge(Some(HandoffTarget::Route));
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend_addr))
        .plugin(plugin)
        .plugin(recorder.plugin())
        .patch_config(receive_proxy_protocol)
        .start()
        .await
        .unwrap();
    ban_ip(&proxy, "203.0.113.7").await;

    let mut sent = proxy_header("203.0.113.7", 50007);
    sent.extend(handshake("lobby.test", CURRENT, ConnectionState::Login));
    sent.extend(frame(&declared_login_start("xferSteve", CURRENT)));
    let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
    client.write_all(&sent).await.unwrap();

    let reason = login_disconnect(client).await;
    assert!(reason.contains("griefing"), "{reason}");
    nothing_accepted(&backend).await;
    assert_eq!(claims.lock().unwrap().len(), 1);

    settle(&proxy).await;
    assert_eq!(
        rejections(&recorder),
        [
            json!({ "remote_addr": "203.0.113.7:50007", "virtual_host": "lobby.test", "reason": "ip_banned", "plugin": null }),
        ]
    );
    assert_eq!(recorder.count(EventKind::PreLogin), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_handshake_handoff_from_a_banned_address_is_refused_like_a_banned_login() {
    let (backend, backend_addr) = listener().await;
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend_addr))
        .plugin(handshake_gate(ConnectionState::Login))
        .plugin(recorder.plugin())
        .patch_config(receive_proxy_protocol)
        .start()
        .await
        .unwrap();
    ban_ip(&proxy, "203.0.113.7").await;

    let mut sent = proxy_header("203.0.113.7", 50008);
    sent.extend(handshake("lobby.test", CURRENT, ConnectionState::Login));
    sent.extend(frame(&declared_login_start("Steve", CURRENT)));
    let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
    client.write_all(&sent).await.unwrap();
    let reason = login_disconnect(client).await;
    assert!(reason.contains("griefing"), "{reason}");
    nothing_accepted(&backend).await;

    settle(&proxy).await;
    assert_eq!(
        rejections(&recorder),
        [
            json!({ "remote_addr": "203.0.113.7:50008", "virtual_host": "lobby.test", "reason": "ip_banned", "plugin": null }),
        ]
    );
    assert_eq!(recorder.count(EventKind::ConnectionHandshake), 1);
    assert_eq!(recorder.count(EventKind::PreLogin), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handoffs_are_refused_while_the_ban_provider_cannot_answer() {
    let (backend, backend_addr) = listener().await;
    let (plugin, claims) = login_bridge(Some(HandoffTarget::Route));
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend_addr))
        .plugin(plugin)
        .plugin(handshake_gate(ConnectionState::Status))
        .plugin(recorder.plugin())
        .patch_config(|table| {
            receive_proxy_protocol(table);
            select_ban_provider(table, "absent");
        })
        .start()
        .await
        .unwrap();

    let mut sent = proxy_header("203.0.113.9", 50010);
    sent.extend(handshake("lobby.test", CURRENT, ConnectionState::Status));
    sent.extend(frame(&[]));
    let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
    client.write_all(&sent).await.unwrap();
    closed_silently(&mut client).await;

    let mut sent = proxy_header("203.0.113.9", 50011);
    sent.extend(handshake("lobby.test", CURRENT, ConnectionState::Login));
    sent.extend(frame(&declared_login_start("xferSteve", CURRENT)));
    let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
    client.write_all(&sent).await.unwrap();
    let reason = login_disconnect(client).await;
    assert!(reason.contains("cannot be checked"), "{reason}");

    nothing_accepted(&backend).await;
    assert_eq!(claims.lock().unwrap().len(), 1);
    settle(&proxy).await;
    assert_eq!(
        rejections(&recorder),
        [
            json!({ "remote_addr": "203.0.113.9:50010", "virtual_host": "lobby.test", "reason": "banned", "plugin": null }),
            json!({ "remote_addr": "203.0.113.9:50011", "virtual_host": "lobby.test", "reason": "banned", "plugin": null }),
        ]
    );
    assert_eq!(recorder.count(EventKind::ProxyPing), 0);
    assert_eq!(recorder.count(EventKind::PreLogin), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_passed_claim_goes_on_as_minecraft_with_its_bytes() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let domain = format!("lobby.test\0{}", "\u{20ac}".repeat(200));
    let opening = handshake(&domain, CURRENT, ConnectionState::Status);
    let prefix = opening[..2].to_vec();
    validate_claim_prefix(&prefix).unwrap();
    let (plugin, seen) = bridge(vec![prefix.clone()], RawConnectionEvent::pass);
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .start()
        .await
        .unwrap();

    let status = FakeClient::new(proxy.addr(), ProtocolVersion(CURRENT))
        .domain(domain.clone())
        .status()
        .await
        .unwrap();
    assert!(status.json.get("description").is_some(), "{status:?}");

    let mut sent = handshake_to(
        &domain,
        proxy.addr().port(),
        CURRENT,
        ConnectionState::Status,
    );
    sent.extend(frame(&[]));
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, Bytes::from(prefix));
    assert!(
        seen[0].1.len() >= 2 && sent.starts_with(&seen[0].1),
        "{:?} is not the start of {sent:?}",
        seen[0].1
    );

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_handshake_handoff_relays_a_login_byte_for_byte() {
    let (backend, backend_addr) = listener().await;
    let recorder = Recorder::new();
    let gate = ScriptedPlugin::new(BRIDGE).on::<ConnectionHandshakeEvent>(
        EventPriority::NORMAL,
        |event| {
            if event.raw_host.ends_with("\0FML3\0") {
                event.handoff(Handoff::route());
            }
        },
    );
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend_addr))
        .plugin(gate)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut sent = handshake("lobby.test\0FML3\0", 767, ConnectionState::Login);
    sent.extend(frame(&declared_login_start("Steve", 767)));
    sent.extend_from_slice(PIPELINED);
    let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
    client.write_all(&sent).await.unwrap();

    let mut upstream = accept(&backend).await;
    assert_eq!(read_n(&mut upstream, sent.len()).await, sent);
    both_ways(&mut client, &mut upstream).await;

    settle(&proxy).await;
    assert_eq!(recorder.count(EventKind::ConnectionHandshake), 1);
    assert_eq!(recorder.count(EventKind::PreLogin), 0);
    assert!(rejections(&recorder).is_empty());

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_handshake_handoff_drops_forwarding_fields_the_client_appended_to_its_host() {
    let (backend, backend_addr) = listener().await;
    let gate = ScriptedPlugin::new(BRIDGE).on::<ConnectionHandshakeEvent>(
        EventPriority::NORMAL,
        |event| {
            if event.raw_host.contains('\0') {
                event.handoff(Handoff::route());
            }
        },
    );
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::passthrough("lobby")
                .backend(backend_addr)
                .patch(|table| {
                    table.insert(
                        "forwarding_mode".into(),
                        toml::Value::String("legacy".into()),
                    );
                }),
        )
        .plugin(gate)
        .start()
        .await
        .unwrap();

    let smuggled = "lobby.test\x00198.51.100.7\x00069a79f444e94726a5befca90e38aaf5\x00[]";
    let login_start = frame(&declared_login_start("Steve", 767));
    let mut sent = handshake(smuggled, 767, ConnectionState::Login);
    sent.extend_from_slice(&login_start);
    sent.extend_from_slice(PIPELINED);
    let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
    client.write_all(&sent).await.unwrap();

    let mut upstream = accept(&backend).await;
    let expected = handshake("lobby.test", 767, ConnectionState::Login);
    assert_eq!(read_n(&mut upstream, expected.len()).await, expected);
    assert_eq!(read_n(&mut upstream, login_start.len()).await, login_start);
    assert_eq!(read_n(&mut upstream, PIPELINED.len()).await, PIPELINED);
    both_ways(&mut client, &mut upstream).await;

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_handshake_handoff_relays_a_status_ping() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (files, files_addr) = listener().await;
    let gate = ScriptedPlugin::new(BRIDGE).on::<ConnectionHandshakeEvent>(
        EventPriority::NORMAL,
        move |event| {
            event.handoff(
                Handoff::to(HandoffTarget::address("127.0.0.1", files_addr.port()))
                    .with_reply(&b"hi"[..]),
            );
        },
    );
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(gate)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut sent = handshake("lobby.test", CURRENT, ConnectionState::Status);
    sent.extend(frame(&[]));
    let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
    client.write_all(&sent).await.unwrap();

    let mut upstream = accept(&files).await;
    assert_eq!(read_n(&mut upstream, sent.len()).await, sent);
    assert_eq!(read_n(&mut client, 2).await, b"hi");
    both_ways(&mut client, &mut upstream).await;

    settle(&proxy).await;
    assert_eq!(recorder.count(EventKind::ProxyPing), 0);
    assert_eq!(backend.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

struct Claim {
    protocol: i32,
    layout_matches: bool,
    login_start: Bytes,
    server: Option<String>,
}

type Claims = Arc<Mutex<Vec<Claim>>>;

fn login_bridge(target: Option<HandoffTarget>) -> (ScriptedPlugin, Claims) {
    let claims: Claims = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&claims);
    let plugin =
        ScriptedPlugin::new(BRIDGE).on::<LoginClaimEvent>(EventPriority::NORMAL, move |event| {
            record.lock().unwrap().push(Claim {
                protocol: event.protocol_version.raw(),
                layout_matches: event.layout_matches,
                login_start: event.login_start.clone(),
                server: event.server.as_ref().map(|s| s.as_str().to_string()),
            });
            if let Some(target) = &target
                && event.username.starts_with("xfer")
            {
                event.handoff(Handoff::to(target.clone()));
            }
        });
    (plugin, claims)
}

async fn modern_layout_logins_are_claimed(spec: ServerSpec) {
    let (backend, backend_addr) = listener().await;
    let (plugin, claims) = login_bridge(Some(HandoffTarget::Route));
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(spec.backend(backend_addr))
        .plugin(plugin)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    for version in MODERN_LAYOUT_VERSIONS {
        let login_start = modern_layout_login_start(&format!("xfer{version}"));
        let mut sent = handshake("lobby.test", version, ConnectionState::Login);
        sent.extend(frame(&login_start));
        sent.extend_from_slice(PIPELINED);
        let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
        client.write_all(&sent).await.unwrap();

        let mut upstream = accept(&backend).await;
        assert_eq!(
            read_n(&mut upstream, sent.len()).await,
            sent,
            "protocol {version}"
        );
        both_ways(&mut client, &mut upstream).await;

        let claims = claims.lock().unwrap();
        let claim = claims.last().unwrap();
        assert_eq!(claim.protocol, version);
        assert_eq!(claim.layout_matches, version >= 764, "protocol {version}");
        assert_eq!(claim.login_start, Bytes::from(login_start));
        assert_eq!(claim.server.as_deref(), Some("lobby"));
    }

    settle(&proxy).await;
    assert_eq!(recorder.count(EventKind::PreLogin), 0);
    assert_eq!(recorder.count(EventKind::ConnectionHandshake), 2);
    assert!(rejections(&recorder).is_empty());
    assert_eq!(proxy.connection_count(), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn modern_layout_logins_are_claimed_and_relayed_on_offline_routes() {
    modern_layout_logins_are_claimed(ServerSpec::offline("lobby")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn modern_layout_logins_are_claimed_and_relayed_on_client_only_routes() {
    modern_layout_logins_are_claimed(ServerSpec::client_only("lobby")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn modern_layout_logins_are_claimed_and_relayed_on_passthrough_routes() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (files, files_addr) = listener().await;
    let (plugin, claims) = login_bridge(Some(HandoffTarget::server("files")));
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
        .server(ServerSpec::passthrough("files").backend(files_addr))
        .plugin(plugin)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    for version in MODERN_LAYOUT_VERSIONS {
        let mut sent = handshake("lobby.test", version, ConnectionState::Login);
        sent.extend(frame(&modern_layout_login_start("xfer")));
        sent.extend_from_slice(PIPELINED);
        let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
        client.write_all(&sent).await.unwrap();

        let mut upstream = accept(&files).await;
        assert_eq!(
            read_n(&mut upstream, sent.len()).await,
            sent,
            "protocol {version}"
        );
        both_ways(&mut client, &mut upstream).await;
    }

    assert_eq!(claims.lock().unwrap().len(), MODERN_LAYOUT_VERSIONS.len());
    settle(&proxy).await;
    assert_eq!(recorder.count(EventKind::PreLogin), 0);
    assert_eq!(backend.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

async fn unclaimed_mismatched_logins_are_still_refused(spec: ServerSpec) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (plugin, claims) = login_bridge(None);
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(spec.backend(backend.addr()))
        .plugin(plugin)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut sent = handshake("lobby.test", 763, ConnectionState::Login);
    sent.extend(frame(&modern_layout_login_start("xfer")));
    let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
    client.write_all(&sent).await.unwrap();
    closed_silently(&mut client).await;

    {
        let claims = claims.lock().unwrap();
        assert_eq!(claims.len(), 1);
        assert!(!claims[0].layout_matches);
    }
    settle(&proxy).await;
    assert_eq!(recorder.count(EventKind::PreLogin), 0);
    assert_eq!(backend.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unclaimed_mismatched_login_is_still_refused_on_offline_routes() {
    unclaimed_mismatched_logins_are_still_refused(ServerSpec::offline("lobby")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unclaimed_mismatched_login_is_still_refused_on_client_only_routes() {
    unclaimed_mismatched_logins_are_still_refused(ServerSpec::client_only("lobby")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unclaimed_login_goes_through_the_normal_login() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (plugin, claims) = login_bridge(Some(HandoffTarget::Route));
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let session = proxy
        .client(ProtocolVersion(CURRENT))
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();

    {
        let claims = claims.lock().unwrap();
        assert_eq!(claims.len(), 1);
        assert!(claims[0].layout_matches);
    }
    assert_eq!(recorder.count(EventKind::PreLogin), 1);

    session.quit().await;
    proxy.shutdown().await.unwrap();
}
