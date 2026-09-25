#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_api::error::PlayerError;
use infrarust_api::event::EventPriority;
use infrarust_api::events::lifecycle::PreLoginEvent;
use infrarust_api::services::ban_service::BanTarget;
use infrarust_api::types::{Component, NamedColor};
use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_test_harness::legacy::LEGACY_PROTOCOL;
use infrarust_test_harness::{
    DEFAULT_TIMEOUT, EventKind, FakeLegacyBackend, LegacyPing, Recorded, Recorder, ScriptedPlugin,
    ServerSpec, TestProxy,
};
use serde_json::json;

const T: Duration = DEFAULT_TIMEOUT;
const NOTCH: &str = "Notch";

fn backend_ping() -> LegacyPing {
    LegacyPing {
        protocol: Some(i32::from(LEGACY_PROTOCOL)),
        version: Some("1.6.4".into()),
        motd: "A legacy backend".into(),
        online: 3,
        max: 20,
    }
}

fn named(event: &Recorded, username: &str) -> bool {
    event.username.as_deref() == Some(username)
}

async fn legacy_proxy(backend: &FakeLegacyBackend, plugins: Vec<ScriptedPlugin>) -> TestProxy {
    let mut builder =
        TestProxy::builder().server(ServerSpec::passthrough("old").backend(backend.addr()));
    for plugin in plugins {
        builder = builder.plugin(plugin);
    }
    builder.start().await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_legacy_login_is_a_registered_player() {
    let backend = FakeLegacyBackend::spawn(backend_ping()).await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("old").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let session = proxy
        .legacy_client_for("old")
        .unwrap()
        .login(NOTCH)
        .await
        .unwrap()
        .forwarded()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), NOTCH);
    assert_eq!(conn.handshake().hostname, "old.test");
    assert_eq!(conn.handshake().protocol, LEGACY_PROTOCOL);

    let player = proxy.wait_for_player(NOTCH, T).await.unwrap();
    assert!(!player.is_active());
    assert!(matches!(
        player.send_message(Component::text("hi")),
        Err(PlayerError::NotActive)
    ));
    assert_eq!(player.profile().uuid, offline_uuid(NOTCH));
    assert_eq!(player.protocol_version().raw(), i32::from(LEGACY_PROTOCOL));
    assert_eq!(
        player
            .current_server()
            .as_ref()
            .map(|s| s.as_str().to_string()),
        Some("old".to_string())
    );
    drop(player);

    session.quit().await;
    conn.closed(T).await.unwrap();
    conn.close().await;
    let disconnect = recorder
        .wait_for(|e| e.kind == EventKind::Disconnect && named(e, NOTCH), T)
        .await
        .unwrap();
    proxy.wait_for_connection_count(0, T).await.unwrap();

    let kinds: Vec<EventKind> = recorder
        .for_username(NOTCH)
        .iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            EventKind::PreLogin,
            EventKind::GameProfileRequest,
            EventKind::PermissionsSetup,
            EventKind::Login,
            EventKind::PostLogin,
            EventKind::PlayerChooseInitialServer,
            EventKind::ServerPreConnect,
            EventKind::ServerConnected,
            EventKind::Disconnect,
        ]
    );
    let pre_login = &recorder.of(EventKind::PreLogin)[0];
    assert_eq!(pre_login.detail["server_domain"], json!("old.test"));
    assert_eq!(
        pre_login.detail["protocol_version"],
        json!(i32::from(LEGACY_PROTOCOL))
    );
    let post_login = &recorder.of(EventKind::PostLogin)[0];
    assert_eq!(post_login.player, disconnect.player);
    assert_forwarding_ended(&disconnect);
    assert_eq!(disconnect.detail["last_server"], json!("old"));

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_banned_name_cannot_log_in_with_a_legacy_client() {
    let backend = FakeLegacyBackend::spawn(backend_ping()).await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("old").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    let target = BanTarget::Username("Griefer".into());
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

    let reason = proxy
        .legacy_client_for("old")
        .unwrap()
        .login("Griefer")
        .await
        .unwrap()
        .kicked()
        .unwrap();

    assert_eq!(reason, kick);
    assert_eq!(recorder.count(EventKind::PostLogin), 0);
    assert_eq!(recorder.count(EventKind::Disconnect), 0);
    assert_eq!(proxy.connection_count(), 0);

    let session = proxy
        .legacy_client_for("old")
        .unwrap()
        .login(NOTCH)
        .await
        .unwrap()
        .forwarded()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), NOTCH);
    session.quit().await;
    conn.closed(T).await.unwrap();
    conn.close().await;

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pre_login_denial_reaches_a_legacy_client_as_legacy_text() {
    let backend = FakeLegacyBackend::spawn(backend_ping()).await.unwrap();
    let recorder = Recorder::new();
    let gate = ScriptedPlugin::new("gate").on::<PreLoginEvent>(EventPriority::NORMAL, |event| {
        event.deny(Component::text("Old clients stay out").color(NamedColor::Red));
    });
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("old").backend(backend.addr()))
        .plugin(gate)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let reason = proxy
        .legacy_client_for("old")
        .unwrap()
        .login(NOTCH)
        .await
        .unwrap()
        .kicked()
        .unwrap();

    assert_eq!(reason, "\u{a7}cOld clients stay out");
    let pre_login = recorder
        .wait_for(|e| e.kind == EventKind::PreLogin && named(e, NOTCH), T)
        .await
        .unwrap();
    assert_eq!(
        pre_login.detail["result"],
        json!({ "denied": "Old clients stay out" })
    );
    assert_eq!(recorder.count(EventKind::PostLogin), 0);
    assert_eq!(recorder.count(EventKind::Disconnect), 0);
    assert_eq!(proxy.connection_count(), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_legacy_ping_is_relayed_from_the_backend() {
    let backend = FakeLegacyBackend::spawn(backend_ping()).await.unwrap();
    let proxy = legacy_proxy(&backend, Vec::new()).await;

    let ping = proxy
        .legacy_client_for("old")
        .unwrap()
        .ping()
        .await
        .unwrap();

    assert_eq!(ping, backend_ping());
    proxy.shutdown().await.unwrap();
}

fn assert_forwarding_ended(disconnect: &Recorded) {
    let cause = &disconnect.detail["cause"];
    assert!(
        *cause == json!("client_quit") || *cause == json!("backend_closed"),
        "{disconnect:?}"
    );
    assert_eq!(disconnect.detail["reason"], json!(null), "{disconnect:?}");
}
