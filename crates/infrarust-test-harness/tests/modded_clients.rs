#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_config::ProxyMode;
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::{McBufWriteExt, Packet, PacketEncoder, VarInt};
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, FakeBackend, ProtocolVersion, ServerSpec, TestProxy,
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use uuid::Uuid;

const T: Duration = DEFAULT_TIMEOUT;
const STEVE: &str = "Steve";
const MARKED_UUID: Uuid = Uuid::from_u128(0xa7c3_5e10_9b2d_4f6e_8a01_c4d2_e3f4_a5b6);
const PIPELINED: &[u8] = b"\x16\x03\x01\x00\x05hello";
const MISMATCHED_VERSIONS: [i32; 4] = [759, 760, 761, 763];

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut encoder = PacketEncoder::new();
    encoder.append_raw(0x00, payload).unwrap();
    encoder.take().to_vec()
}

fn modern_layout_opening(domain: &str, version: i32, name: &str) -> Vec<u8> {
    let handshake = SHandshake {
        protocol_version: VarInt(version),
        server_address: domain.to_string(),
        server_port: 25565,
        next_state: ConnectionState::Login,
        transfer: false,
    };
    let mut handshake_payload = Vec::new();
    handshake
        .encode(&mut handshake_payload, ProtocolVersion(version))
        .unwrap();
    let mut login_payload = Vec::new();
    login_payload.write_string(name).unwrap();
    let uuid = Uuid::from_u128(MARKED_UUID.as_u128() + u128::from(version.unsigned_abs()));
    login_payload.write_uuid(&uuid).unwrap();

    let mut opening = frame(&handshake_payload);
    opening.extend(frame(&login_payload));
    opening
}

async fn a_mismatched_login_start_is_forwarded_byte_for_byte(mode: ProxyMode) {
    let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::new("lobby", mode).backend(backend.local_addr().unwrap()))
        .start()
        .await
        .unwrap();
    let domain = proxy.domain("lobby").unwrap().to_string();

    for version in MISMATCHED_VERSIONS {
        let mut sent = modern_layout_opening(&domain, version, &format!("amp{version}"));
        sent.extend_from_slice(PIPELINED);
        let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
        client.write_all(&sent).await.unwrap();

        let (mut upstream, _) = timeout(T, backend.accept()).await.unwrap().unwrap();
        let mut received = vec![0; sent.len()];
        timeout(T, upstream.read_exact(&mut received))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, sent, "protocol {version}");

        upstream.write_all(b"pong").await.unwrap();
        let mut reply = [0; 4];
        timeout(T, client.read_exact(&mut reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&reply, b"pong");
    }

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mismatched_login_start_is_forwarded_byte_for_byte_in_passthrough() {
    a_mismatched_login_start_is_forwarded_byte_for_byte(ProxyMode::Passthrough).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mismatched_login_start_is_forwarded_byte_for_byte_in_zero_copy() {
    a_mismatched_login_start_is_forwarded_byte_for_byte(ProxyMode::ZeroCopy).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mismatched_login_start_is_forwarded_byte_for_byte_in_server_only() {
    a_mismatched_login_start_is_forwarded_byte_for_byte(ProxyMode::ServerOnly).await;
}

async fn a_mismatched_login_start_is_refused(spec: ServerSpec) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(spec.backend(backend.addr()))
        .start()
        .await
        .unwrap();
    let domain = proxy.domain("lobby").unwrap().to_string();

    for version in MISMATCHED_VERSIONS {
        let mut client = TcpStream::connect(proxy.addr()).await.unwrap();
        client
            .write_all(&modern_layout_opening(&domain, version, "amp"))
            .await
            .unwrap();
        let mut rest = Vec::new();
        let read = timeout(T, client.read_to_end(&mut rest)).await.unwrap();
        assert!(
            read.is_err() || rest.is_empty(),
            "protocol {version}: {rest:?}"
        );
    }
    assert_eq!(backend.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mismatched_login_start_is_refused_in_offline() {
    a_mismatched_login_start_is_refused(ServerSpec::offline("lobby")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mismatched_login_start_is_refused_in_client_only() {
    a_mismatched_login_start_is_refused(ServerSpec::client_only("lobby")).await;
}

async fn forwarded_host(spec: ServerSpec, backend: &FakeBackend, marker: &str) -> String {
    let proxy = TestProxy::builder().server(spec).start().await.unwrap();
    let domain = format!("{}{marker}", proxy.domain("lobby").unwrap());
    let session = proxy
        .client(ProtocolVersion(763))
        .domain(domain)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    let host = conn.handshake().server_address.clone();
    session.quit().await;
    conn.closed(T).await.unwrap();
    conn.close().await;
    proxy.shutdown().await.unwrap();
    host
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_forge_marker_survives_a_domain_rewrite() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let spec = ServerSpec::passthrough("lobby")
        .backend(backend.addr())
        .patch(|table| {
            let mut rewrite = toml::Table::new();
            rewrite.insert(
                "explicit".into(),
                toml::Value::String("backend.example".into()),
            );
            table.insert("domain_rewrite".into(), toml::Value::Table(rewrite));
        });
    assert_eq!(
        forwarded_host(spec, &backend, "\0FML3\0").await,
        "backend.example\0FML3\0"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_forge_marker_reaches_a_bungeecord_backend_as_properties() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let spec = ServerSpec::passthrough("lobby")
        .backend(backend.addr())
        .patch(|table| {
            table.insert(
                "forwarding_mode".into(),
                toml::Value::String("legacy".into()),
            );
        });
    let host = forwarded_host(spec, &backend, "\0FML3\0").await;

    let parts: Vec<&str> = host.split('\0').collect();
    assert_eq!(parts.len(), 4, "{parts:?}");
    assert_eq!(parts[0], "lobby.test");
    assert_eq!(parts[1], "127.0.0.1");
    let properties: Vec<Value> = serde_json::from_str(parts[3]).unwrap();
    assert!(
        properties
            .iter()
            .any(|p| p["name"] == "forgeClient" && p["value"] == "true"),
        "{properties:?}"
    );
    assert!(
        properties
            .iter()
            .any(|p| p["name"] == "extraData" && p["value"] == "\u{1}FML3\u{1}"),
        "{properties:?}"
    );
}
