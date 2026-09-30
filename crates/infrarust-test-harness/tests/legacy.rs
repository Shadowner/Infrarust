#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_api::error::PlayerError;
use infrarust_api::event::EventPriority;
use infrarust_api::events::lifecycle::PreLoginEvent;
use infrarust_api::services::ban_service::{BanRequest, BanTarget};
use infrarust_api::types::{Component, NamedColor};
use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_test_harness::legacy::LEGACY_PROTOCOL;
use infrarust_test_harness::{
    DEFAULT_TIMEOUT, EventKind, FakeLegacyBackend, HarnessError, LegacyClient, LegacyPing,
    Recorded, Recorder, ScriptedPlugin, ServerSpec, TestProxy,
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
        .wait_for(|e| e.kind == EventKind::Disconnect && e.is_named(NOTCH), T)
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
    assert_eq!(pre_login.server_domain(), Some("old.test"));
    assert_eq!(pre_login.protocol_version(), i32::from(LEGACY_PROTOCOL));
    let post_login = &recorder.of(EventKind::PostLogin)[0];
    assert_eq!(post_login.player, disconnect.player);
    assert_forwarding_ended(&disconnect);
    assert_eq!(disconnect.last_server(), Some("old"));

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
    bans.issue(BanRequest::new(target.clone()).reason("griefing"))
        .await
        .unwrap();
    let kick = bans
        .get(&target)
        .await
        .unwrap()
        .unwrap()
        .default_kick_message()
        .to_plain();

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
        .wait_for(|e| e.kind == EventKind::PreLogin && e.is_named(NOTCH), T)
        .await
        .unwrap();
    assert_eq!(
        *pre_login.result(),
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

fn closed_without_answer<T: std::fmt::Debug>(result: Result<T, HarnessError>) {
    match result {
        Err(HarnessError::Closed(_) | HarnessError::Io(_)) => {}
        other => panic!("expected the proxy to close the connection silently, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_domain_drops_legacy_clients_when_asked() {
    let backend = FakeLegacyBackend::spawn(backend_ping()).await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("old").backend(backend.addr()))
        .plugin(recorder.plugin())
        .patch_config(|table| {
            table.insert("unknown_domain_behavior".into(), "drop".into());
        })
        .start()
        .await
        .unwrap();
    let stranger = LegacyClient::new(proxy.addr()).hostname("nowhere.test");

    closed_without_answer(stranger.ping().await);
    closed_without_answer(stranger.ping_beta().await);
    closed_without_answer(stranger.login(NOTCH).await);
    let known = proxy
        .legacy_client_for("old")
        .unwrap()
        .ping()
        .await
        .unwrap();
    assert_eq!(known, backend_ping());

    tokio::time::timeout(T, proxy.bus().flush())
        .await
        .expect("the event queue never drained");
    let rejected: Vec<(Option<String>, String)> = recorder
        .of(EventKind::ConnectionRejected)
        .iter()
        .map(|e| {
            (
                e.virtual_host().map(str::to_string),
                e.detail_str("reason").to_string(),
            )
        })
        .collect();
    assert_eq!(
        rejected,
        [
            (Some("nowhere.test".into()), "unknown_domain".into()),
            (None, "unknown_domain".into()),
            (Some("nowhere.test".into()), "unknown_domain".into()),
        ]
    );
    assert_eq!(recorder.count(EventKind::ConnectionHandshake), 1);
    assert_eq!(recorder.count(EventKind::ProxyPing), 1);
    assert_eq!(recorder.count(EventKind::PreLogin), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_domain_answers_legacy_clients_by_default() {
    let backend = FakeLegacyBackend::spawn(backend_ping()).await.unwrap();
    let proxy = legacy_proxy(&backend, Vec::new()).await;
    let stranger = LegacyClient::new(proxy.addr()).hostname("nowhere.test");

    assert_eq!(stranger.ping().await.unwrap().motd, "An Infrarust Proxy");
    assert_eq!(
        stranger.ping_beta().await.unwrap().motd,
        "An Infrarust Proxy"
    );
    assert_eq!(
        stranger.login(NOTCH).await.unwrap().kicked().unwrap(),
        "Unknown server"
    );

    proxy.shutdown().await.unwrap();
}

fn assert_forwarding_ended(disconnect: &Recorded) {
    assert_eq!(disconnect.cause(), "client_quit", "{disconnect:?}");
    assert_eq!(disconnect.reason(), None, "{disconnect:?}");
}
