#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_protocol::packets::config::{CFinishConfig, SAcknowledgeFinishConfig};
use infrarust_protocol::packets::play::chat::{SChatCommand, SChatMessage};
use infrarust_protocol::packets::play::keepalive::{CKeepAlive, SKeepAlive};
use infrarust_protocol::packets::play::start_configuration::{
    CStartConfiguration, SAcknowledgeConfiguration,
};
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, FakeBackend, FakeClient, HarnessError, LoginBehavior,
    ProtocolVersion, version_matrix,
};
use serde_json::json;

const T: Duration = DEFAULT_TIMEOUT;

async fn direct_lifecycle(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let client = FakeClient::new(backend.addr(), version).domain("lobby.test");

    let mut session = client.login("Steve").await.unwrap().joined().unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();

    assert_eq!(conn.username(), "Steve");
    let handshake = conn.handshake();
    assert_eq!(handshake.protocol_version, version.0);
    assert_eq!(handshake.server_address, "lobby.test");
    assert_eq!(handshake.server_port, backend.addr().port());
    assert_eq!(handshake.next_state, 2);
    let expected_uuid = offline_uuid("Steve");
    if version.no_less_than(ProtocolVersion::V1_19_1) {
        assert_eq!(conn.uuid(), Some(expected_uuid));
    } else {
        assert_eq!(conn.uuid(), None);
    }
    assert_eq!(session.uuid(), Some(expected_uuid));

    session.chat("hi").await.unwrap();
    let chat = conn.expect::<SChatMessage>(T).await.unwrap();
    assert_eq!(chat.message, "hi");

    session.command("spawn").await.unwrap();
    if version.less_than(ProtocolVersion::V1_19) {
        assert_eq!(
            conn.expect::<SChatMessage>(T).await.unwrap().message,
            "/spawn"
        );
    } else {
        assert_eq!(
            conn.expect::<SChatCommand>(T).await.unwrap().command,
            "spawn"
        );
    }

    conn.send_packet(&CKeepAlive { id: 42 }).await.unwrap();
    assert_eq!(conn.expect::<SKeepAlive>(T).await.unwrap().id, 42);

    conn.send_system_message_json(r#"{"text":"Welcome","extra":[{"text":" back"}]}"#)
        .await
        .unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "Welcome back");

    conn.kick_json(r#"{"text":"Server closed","color":"red"}"#)
        .await
        .unwrap();
    let info = session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.state, ConnectionState::Play);
    assert_eq!(info.text, "Server closed");
    assert_eq!(
        info.json.is_some(),
        version.less_than(ProtocolVersion::V1_20_3)
    );
    conn.closed(T).await.unwrap();
}

version_matrix!(LIFECYCLE, direct_lifecycle);

async fn direct_status(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let status = FakeClient::new(backend.addr(), version)
        .domain("lobby.test")
        .status()
        .await
        .unwrap();
    assert_eq!(status.json["version"]["protocol"], json!(version.0));
    assert_eq!(status.json["description"]["text"], "Infrarust fake backend");
    assert_eq!(backend.status_requests(), 1);
}

version_matrix!(LIFECYCLE, direct_status);

async fn compressed_login(version: ProtocolVersion) {
    let backend = FakeBackend::builder()
        .compression(Some(256))
        .spawn()
        .await
        .unwrap();
    let client = FakeClient::new(backend.addr(), version);
    let mut session = client.login("Alex").await.unwrap().joined().unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), "Alex");

    let long = "x".repeat(256);
    session.chat(&long).await.unwrap();
    assert_eq!(conn.expect::<SChatMessage>(T).await.unwrap().message, long);

    let long_json = json!({ "text": "y".repeat(600) }).to_string();
    conn.send_system_message_json(&long_json).await.unwrap();
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "y".repeat(600)
    );
}

version_matrix!(LIFECYCLE, compressed_login);

async fn refused_login(version: ProtocolVersion) {
    let backend = FakeBackend::builder()
        .login(LoginBehavior::refuse_text("Whitelist only"))
        .spawn()
        .await
        .unwrap();
    let info = FakeClient::new(backend.addr(), version)
        .login("Steve")
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(info.state, ConnectionState::Login);
    assert_eq!(info.text, "Whitelist only");
    assert_eq!(info.json, Some(json!({ "text": "Whitelist only" })));

    let conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), "Steve");
    assert_eq!(conn.state(), ConnectionState::Login);
}

version_matrix!(LIFECYCLE, refused_login);

async fn rejoin_after_reconfiguration(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let mut session = FakeClient::new(backend.addr(), version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();

    if version.no_less_than(ProtocolVersion::V1_20_2) {
        conn.send_packet(&CStartConfiguration).await.unwrap();
        conn.expect::<SAcknowledgeConfiguration>(T).await.unwrap();
        conn.send_packet(&CFinishConfig).await.unwrap();
        conn.expect::<SAcknowledgeFinishConfig>(T).await.unwrap();
    }
    let join = infrarust_core::test_support::join_game_frame(version).unwrap();
    conn.send_frame(&join).await.unwrap();
    let rejoined = session.expect_join(T).await.unwrap();
    assert_eq!(rejoined.id, join.id);
    assert_eq!(rejoined.payload, join.payload);

    session.chat("still here").await.unwrap();
    assert_eq!(
        conn.expect::<SChatMessage>(T).await.unwrap().message,
        "still here"
    );
}

version_matrix!(SWITCH, rejoin_after_reconfiguration);

#[tokio::test]
async fn hanging_backend_times_out_the_login() {
    let backend = FakeBackend::builder()
        .login(LoginBehavior::Hang)
        .spawn()
        .await
        .unwrap();
    let result = FakeClient::new(backend.addr(), ProtocolVersion::V1_21_11)
        .timeout(Duration::from_millis(300))
        .login("Steve")
        .await;
    assert!(
        matches!(result, Err(HarnessError::Timeout { .. })),
        "{result:?}"
    );
    let conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.state(), ConnectionState::Login);
    conn.closed(T).await.unwrap();
}

async fn chat_and_command(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let session = FakeClient::new(backend.addr(), version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();

    session.chat("hello there").await.unwrap();
    assert_eq!(
        conn.expect::<SChatMessage>(T).await.unwrap().message,
        "hello there"
    );

    session.command("/tp 0 64 0").await.unwrap();
    if version.less_than(ProtocolVersion::V1_19) {
        assert_eq!(
            conn.expect::<SChatMessage>(T).await.unwrap().message,
            "/tp 0 64 0"
        );
    } else {
        assert_eq!(
            conn.expect::<SChatCommand>(T).await.unwrap().command,
            "tp 0 64 0"
        );
    }
}

version_matrix!(CHAT, chat_and_command);

async fn nested_kick_text(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let mut session = FakeClient::new(backend.addr(), version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();

    conn.kick_json(r#"{"text":"A","extra":[{"text":"B","extra":[{"text":"C"}]},{"text":"D"}]}"#)
        .await
        .unwrap();
    let info = session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.state, ConnectionState::Play);
    assert_eq!(info.text, "ABCD");
}

version_matrix!(TEXT, nested_kick_text);
