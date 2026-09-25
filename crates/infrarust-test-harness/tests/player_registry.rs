#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use infrarust_api::services::ban_service::BanTarget;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, EventKind, FakeBackend, ProtocolVersion, Recorder,
    ServerSpec, TestProxy,
};
use serde_json::json;
use toml::Value;

const T: Duration = DEFAULT_TIMEOUT;
const VERSION: ProtocolVersion = ProtocolVersion(774);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_username_is_found_in_any_case() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();
    let session = proxy
        .client(VERSION)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let steve = proxy.wait_for_player("Steve", T).await.unwrap();
    let registry = &proxy.services().player_registry;

    for spelling in ["sTeVe", "steve", "STEVE"] {
        let found = registry
            .get_player(spelling)
            .unwrap_or_else(|| panic!("{spelling} must find Steve"));
        assert_eq!(found.id(), steve.id(), "{spelling}");
        assert_eq!(found.profile().username, "Steve");
    }
    assert!(registry.get_player("Steven").is_none());

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_username_ban_in_another_case_kicks_the_player() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    let mut session = proxy
        .client(VERSION)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let conn = backend.next_connection(T).await.unwrap();
    proxy.wait_for_player("Steve", T).await.unwrap();
    let target = BanTarget::Username("sTEVE".into());
    let bans = &proxy.services().ban_manager;

    bans.ban(target.clone(), Some("griefing".into()), None, "test".into())
        .await
        .unwrap();

    let kick = bans
        .is_banned(&target)
        .await
        .unwrap()
        .unwrap()
        .kick_message();
    let info = session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.state, ConnectionState::Play, "{info:?}");
    assert_eq!(info.text, kick, "{info:?}");
    conn.closed(T).await.unwrap();
    let disconnect = recorder
        .wait_for(|e| e.kind == EventKind::Disconnect, T)
        .await
        .unwrap();
    assert_eq!(disconnect.detail["cause"], json!("kicked"));
    assert_eq!(disconnect.detail["reason"], json!(kick));
    proxy.wait_for_connection_count(0, T).await.unwrap();

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn players_behind_the_same_forwarded_address_are_found_together() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .patch_config(|table| {
            table.insert("receive_proxy_protocol".into(), Value::Boolean(true));
        })
        .start()
        .await
        .unwrap();
    let shared: IpAddr = "203.0.113.7".parse().unwrap();
    let other: IpAddr = "198.51.100.9".parse().unwrap();
    let mut sessions = Vec::new();
    for (username, source) in [
        ("Alice", SocketAddr::new(shared, 50001)),
        ("Bob", SocketAddr::new(shared, 50002)),
        ("Carol", SocketAddr::new(other, 50003)),
    ] {
        let session = proxy
            .client(VERSION)
            .proxy_protocol(source)
            .login(username)
            .await
            .unwrap()
            .joined()
            .unwrap();
        proxy.wait_for_player(username, T).await.unwrap();
        sessions.push(session);
    }
    let registry = &proxy.services().player_registry;
    let names = |ip: IpAddr| {
        let mut names: Vec<String> = registry
            .get_players_by_ip(ip)
            .iter()
            .map(|p| p.profile().username.clone())
            .collect();
        names.sort();
        names
    };

    assert_eq!(names(shared), ["Alice", "Bob"]);
    assert_eq!(names(other), ["Carol"]);
    assert!(names("127.0.0.1".parse().unwrap()).is_empty());

    for session in sessions {
        session.quit().await;
    }
    proxy.wait_for_connection_count(0, T).await.unwrap();
    assert!(names(shared).is_empty());
    proxy.shutdown().await.unwrap();
}
