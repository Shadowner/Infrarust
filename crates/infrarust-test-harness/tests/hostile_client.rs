#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use bytes::Bytes;
use infrarust_api::types::Component;
use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_protocol::codec::VarInt;
use infrarust_protocol::io::PacketEncoder;
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::login::{
    CLoginDisconnect, CLoginSuccess, SLoginAcknowledged, SLoginStart,
};
use infrarust_protocol::packets::status::SStatusRequest;
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, DisconnectInfo, FakeBackend, FramedConn, LoginBehavior,
    PacketFrame, ProtocolVersion, ServerSpec, TestProxy, version_matrix, wire,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use toml::{Table, Value};

const T: Duration = DEFAULT_TIMEOUT;
const USERNAME: &str = "Steve";

async fn hung_login(read_timeout: &'static str) -> (FakeBackend, TestProxy) {
    let backend = FakeBackend::builder()
        .login(LoginBehavior::Hang)
        .spawn()
        .await
        .unwrap();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("lobby")
                .backend(backend.addr())
                .patch(move |table| {
                    let mut timeouts = Table::new();
                    timeouts.insert("read".into(), Value::String(read_timeout.into()));
                    table.insert("timeouts".into(), Value::Table(timeouts));
                }),
        )
        .start()
        .await
        .unwrap();
    (backend, proxy)
}

async fn logging_in(proxy: &TestProxy, version: ProtocolVersion) -> FramedConn {
    let mut conn = FramedConn::connect(proxy.addr()).await.unwrap();
    let handshake = SHandshake {
        protocol_version: VarInt(version.0),
        server_address: proxy.domain("lobby").unwrap().to_string(),
        server_port: 25565,
        next_state: ConnectionState::Login,
        transfer: false,
    };
    conn.write_frame(&wire::encode(&handshake, version).unwrap())
        .await
        .unwrap();
    let start = SLoginStart {
        name: USERNAME.to_string(),
        uuid: Some(offline_uuid(USERNAME)),
        profile_key: None,
    };
    conn.write_frame(&wire::encode(&start, version).unwrap())
        .await
        .unwrap();
    conn
}

async fn an_early_acknowledgement_does_not_stop_the_login_timeout(version: ProtocolVersion) {
    let (backend, proxy) = hung_login("1s").await;
    let mut conn = logging_in(&proxy, version).await;
    let _backend_conn = backend.next_connection(T).await.unwrap();

    conn.write_frame(&wire::encode(&SLoginAcknowledged, version).unwrap())
        .await
        .unwrap();

    let frame = tokio::time::timeout(T, conn.read_frame())
        .await
        .expect("the login timeout must still fire")
        .unwrap()
        .expect("the proxy explains why it closes the connection");
    let packet = wire::decode::<CLoginDisconnect>(&frame, version).unwrap();
    let info = DisconnectInfo::from_json_string(ConnectionState::Login, &packet.reason);
    assert_eq!(info.text, "The login took too long", "{info:?}");

    proxy.shutdown().await.unwrap();
}

version_matrix!(an_early_acknowledgement_does_not_stop_the_login_timeout; p764 = 764, p777 = 777);

async fn an_early_acknowledgement_is_not_relayed(version: ProtocolVersion) {
    let (backend, proxy) = hung_login("30s").await;
    let mut conn = logging_in(&proxy, version).await;
    let mut backend_conn = backend.next_connection(T).await.unwrap();

    conn.write_frame(&wire::encode(&SLoginAcknowledged, version).unwrap())
        .await
        .unwrap();
    assert!(
        backend_conn
            .recv_frame(Duration::from_millis(300))
            .await
            .is_err(),
        "an acknowledgement sent before the login succeeded reached the backend"
    );

    let success = CLoginSuccess {
        uuid: offline_uuid(USERNAME),
        username: USERNAME.to_string(),
        properties: Vec::new(),
        strict_error_handling: false,
        session_id: version
            .no_less_than(ProtocolVersion::V26_2)
            .then(uuid::Uuid::new_v4),
    };
    backend_conn.send_packet(&success).await.unwrap();
    let frame = tokio::time::timeout(T, conn.read_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(wire::is::<CLoginSuccess>(&frame, version));

    conn.write_frame(&wire::encode(&SLoginAcknowledged, version).unwrap())
        .await
        .unwrap();
    backend_conn.expect::<SLoginAcknowledged>(T).await.unwrap();

    proxy.shutdown().await.unwrap();
}

version_matrix!(an_early_acknowledgement_is_not_relayed; p764 = 764, p777 = 777);

fn declared_length(length: i32) -> Vec<u8> {
    let mut out = Vec::new();
    VarInt(length).encode(&mut out).unwrap();
    out
}

async fn closed_while_the_declared_bytes_are_missing(mut stream: TcpStream) {
    let mut sink = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut sink))
        .await
        .expect("the proxy waited for a frame over its limit")
        .ok();
    assert!(sink.is_empty(), "{sink:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_handshake_declared_over_its_limit_is_closed_at_once() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let mut stream = TcpStream::connect(proxy.addr()).await.unwrap();
    stream.write_all(&declared_length(2049)).await.unwrap();
    closed_while_the_declared_bytes_are_missing(stream).await;
    assert_eq!(backend.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_login_start_declared_over_its_limit_is_closed_at_once() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let version = ProtocolVersion::V1_21;
    let handshake = wire::encode(
        &SHandshake {
            protocol_version: VarInt(version.0),
            server_address: proxy.domain("lobby").unwrap().to_string(),
            server_port: 25565,
            next_state: ConnectionState::Login,
            transfer: false,
        },
        version,
    )
    .unwrap();
    let mut encoder = PacketEncoder::new();
    encoder
        .append_raw(handshake.id, &handshake.payload)
        .unwrap();
    let mut sent = encoder.take().to_vec();
    sent.extend_from_slice(&declared_length(5121));

    let mut stream = TcpStream::connect(proxy.addr()).await.unwrap();
    stream.write_all(&sent).await.unwrap();
    closed_while_the_declared_bytes_are_missing(stream).await;
    assert_eq!(backend.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

fn status_handshake(proxy: &TestProxy, version: ProtocolVersion) -> Vec<u8> {
    let handshake = wire::encode(
        &SHandshake {
            protocol_version: VarInt(version.0),
            server_address: proxy.domain("lobby").unwrap().to_string(),
            server_port: 25565,
            next_state: ConnectionState::Status,
            transfer: false,
        },
        version,
    )
    .unwrap();
    let mut encoder = PacketEncoder::new();
    encoder
        .append_raw(handshake.id, &handshake.payload)
        .unwrap();
    encoder.take().to_vec()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_status_request_declared_over_its_limit_is_closed_at_once() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let mut sent = status_handshake(&proxy, ProtocolVersion::V1_21);
    sent.extend_from_slice(&declared_length(65));

    let mut stream = TcpStream::connect(proxy.addr()).await.unwrap();
    stream.write_all(&sent).await.unwrap();
    closed_while_the_declared_bytes_are_missing(stream).await;
    assert_eq!(backend.status_requests(), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ping_declared_over_its_limit_is_closed_at_once() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let version = ProtocolVersion::V1_21;
    let request = wire::encode(&SStatusRequest, version).unwrap();
    let mut encoder = PacketEncoder::new();
    encoder.append_raw(request.id, &request.payload).unwrap();
    let mut sent = status_handshake(&proxy, version);
    sent.extend_from_slice(&encoder.take());
    sent.extend_from_slice(&declared_length(65));

    let mut stream = TcpStream::connect(proxy.addr()).await.unwrap();
    stream.write_all(&sent).await.unwrap();
    let mut answered = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut answered))
        .await
        .expect("the proxy waited for a ping over its limit")
        .ok();
    assert!(!answered.is_empty(), "the status request went unanswered");

    proxy.shutdown().await.unwrap();
}

struct StalledClient {
    proxy: TestProxy,
    _backend: FakeBackend,
    _conn: FramedConn,
    flood: JoinHandle<()>,
}

async fn a_client_that_stopped_reading(version: ProtocolVersion) -> StalledClient {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();
    let conn = logging_in(&proxy, version).await;
    let mut backend_conn = backend.next_connection(T).await.unwrap();
    let frame = PacketFrame::new(0x7F, Bytes::from(vec![0; 64 * 1024]));
    let flood = tokio::spawn(async move { while backend_conn.send_frame(&frame).await.is_ok() {} });
    StalledClient {
        proxy,
        _backend: backend,
        _conn: conn,
        flood,
    }
}

async fn a_client_that_stops_reading_loses_its_session_at_the_write_limit(
    version: ProtocolVersion,
) {
    let stalled = a_client_that_stopped_reading(version).await;
    stalled.proxy.wait_for_player(USERNAME, T).await.unwrap();

    let started = tokio::time::Instant::now();
    stalled
        .proxy
        .wait_for_connection_count(0, Duration::from_secs(36))
        .await
        .unwrap();
    assert!(
        started.elapsed() >= Duration::from_secs(25),
        "the session ended after {:?}, before the write limit",
        started.elapsed()
    );

    stalled.flood.abort();
    stalled.proxy.shutdown().await.unwrap();
}

version_matrix!(a_client_that_stops_reading_loses_its_session_at_the_write_limit; p763 = 763);

async fn disconnecting_a_client_that_stopped_reading_ends_within_the_grace(
    version: ProtocolVersion,
) {
    let stalled = a_client_that_stopped_reading(version).await;
    let player = stalled.proxy.wait_for_player(USERNAME, T).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;

    player.disconnect(Component::text("bye")).await;

    stalled
        .proxy
        .wait_for_connection_count(0, Duration::from_secs(8))
        .await
        .unwrap();

    stalled.flood.abort();
    stalled.proxy.shutdown().await.unwrap();
}

version_matrix!(disconnecting_a_client_that_stopped_reading_ends_within_the_grace; p763 = 763);

async fn shutdown_does_not_wait_for_a_client_that_stopped_reading(version: ProtocolVersion) {
    let stalled = a_client_that_stopped_reading(version).await;
    stalled.proxy.wait_for_player(USERNAME, T).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;

    tokio::time::timeout(Duration::from_secs(8), stalled.proxy.shutdown())
        .await
        .expect("shutdown waited on a client that reads nothing")
        .unwrap();

    stalled.flood.abort();
}

version_matrix!(shutdown_does_not_wait_for_a_client_that_stopped_reading; p763 = 763);
