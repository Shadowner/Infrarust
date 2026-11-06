#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_protocol::codec::VarInt;
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::login::{
    CLoginDisconnect, CLoginSuccess, SLoginAcknowledged, SLoginStart,
};
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, DisconnectInfo, FakeBackend, FramedConn, LoginBehavior,
    ProtocolVersion, ServerSpec, TestProxy, version_matrix, wire,
};
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
