#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use infrarust_api::event::EventPriority;
use infrarust_api::event::bus::EventBusExt;
use infrarust_api::events::connection::ServerPreConnectEvent;
use infrarust_api::events::lifecycle::{
    DisconnectEvent, GameProfileRequestEvent, LoginEvent, PostLoginEvent,
};
use infrarust_api::services::ban_service::{BanRequest, BanTarget};
use infrarust_api::types::{Component, ServerId};
use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_core::event_bus::DiagnosticKind;
use infrarust_test_harness::versions::CURRENT;
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, EventKind, FakeBackend, FakeSessionServer, ProtocolVersion,
    Recorded, Recorder, ScriptedPlugin, ServerSpec, TestProxy, TestProxyBuilder, version_matrix,
};
use serde_json::json;
use tokio::sync::mpsc;
use toml::{Table, Value};
use uuid::Uuid;

const T: Duration = DEFAULT_TIMEOUT;

const AWAITED_ORDER: [EventKind; 10] = [
    EventKind::PreLogin,
    EventKind::GameProfileRequest,
    EventKind::PermissionsSetup,
    EventKind::Login,
    EventKind::PostLogin,
    EventKind::PlayerChooseInitialServer,
    EventKind::ServerPreConnect,
    EventKind::ServerConnected,
    EventKind::ServerPostConnect,
    EventKind::Disconnect,
];

fn named(event: &Recorded, username: &str) -> bool {
    event.username.as_deref() == Some(username)
}

fn patch_section(
    builder: TestProxyBuilder,
    section: &'static str,
    entries: Vec<(&'static str, Value)>,
) -> TestProxyBuilder {
    builder.patch_config(move |table| {
        let section = table
            .entry(section)
            .or_insert_with(|| Value::Table(Table::new()))
            .as_table_mut()
            .expect("a config section is a table");
        for (key, value) in entries {
            section.insert(key.into(), value);
        }
    })
}

async fn assert_awaited_order(
    proxy: &TestProxy,
    recorder: &Recorder,
    backend: &FakeBackend,
    version: ProtocolVersion,
    username: &str,
) {
    let session = proxy
        .client(version)
        .login(username)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let conn = backend.next_connection(T).await.unwrap();

    session.quit().await;
    let disconnect = recorder
        .wait_for(|e| e.kind == EventKind::Disconnect && named(e, username), T)
        .await
        .unwrap();
    conn.closed(T).await.unwrap();
    proxy.wait_for_connection_count(0, T).await.unwrap();

    let events = recorder.for_username(username);
    let awaited: Vec<EventKind> = events.iter().map(|e| e.kind).collect();
    assert_eq!(awaited, AWAITED_ORDER, "protocol {}", version.0);
    let player = disconnect.player.expect("Disconnect carries the player");
    for event in events.iter().filter(|e| e.player.is_some()) {
        assert_eq!(event.player, Some(player), "{}", event.kind);
    }
    assert_eq!(disconnect.detail["cause"], json!("client_quit"));
    assert_eq!(disconnect.detail["last_server"], json!("lobby"));
    let post_login = &recorder.of(EventKind::PostLogin)[0];
    assert_eq!(post_login.detail["current_server"], json!(null));
}

async fn offline_events_are_awaited_in_order(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    assert_awaited_order(&proxy, &recorder, &backend, version, "Steve").await;
    let request = &recorder.of(EventKind::GameProfileRequest)[0];
    assert_eq!(request.detail["online_mode"], json!(false));
    assert_eq!(request.detail["virtual_host"], json!("lobby.test"));

    proxy.shutdown().await.unwrap();
}

version_matrix!(LIFECYCLE, offline_events_are_awaited_in_order);

async fn client_only_events_are_awaited_in_order(version: ProtocolVersion) {
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("lobby").backend(backend.addr()))
        .session_server(&sessions)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    assert_awaited_order(&proxy, &recorder, &backend, version, "Alex").await;
    let request = &recorder.of(EventKind::GameProfileRequest)[0];
    assert_eq!(request.detail["online_mode"], json!(true));
    assert_eq!(
        request.detail["profile"]["uuid"],
        json!(offline_uuid("Alex").to_string())
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(client_only_events_are_awaited_in_order;
    p47 = 47, p340 = 340, p763 = 763, p764 = 764, p766 = 766, p774 = 774);

async fn client_only_uuid_ban_ends_in_login(version: ProtocolVersion) {
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("lobby").backend(backend.addr()))
        .session_server(&sessions)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    let target = BanTarget::Uuid(offline_uuid("Banned"));
    let bans = &proxy.services().ban_manager;
    bans.issue(BanRequest::new(target.clone()).reason("cheating"))
        .await
        .unwrap();
    let kick = bans
        .get(&target)
        .await
        .unwrap()
        .unwrap()
        .default_kick_message()
        .to_plain();

    let info = proxy
        .client(version)
        .login("Banned")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(info.state, ConnectionState::Login, "{info:?}");
    assert_eq!(info.text, kick, "{info:?}");
    sessions.wait_for_call("Banned", T).await.unwrap();
    assert_eq!(recorder.count(EventKind::PostLogin), 0);
    assert_eq!(recorder.count(EventKind::Disconnect), 0);
    assert_eq!(proxy.connection_count(), 0);

    proxy.shutdown().await.unwrap();
}

version_matrix!(client_only_uuid_ban_ends_in_login;
    p47 = 47, p340 = 340, p763 = 763, p764 = 764, p766 = 766, p774 = 774);

fn gatekeeper() -> ScriptedPlugin {
    ScriptedPlugin::new("gatekeeper").on::<LoginEvent>(EventPriority::NORMAL, |event| {
        if event.profile().username == "Mallory" {
            event.deny(Component::text("No entry for Mallory"));
        }
    })
}

async fn assert_login_denied(proxy: &TestProxy, recorder: &Recorder, version: ProtocolVersion) {
    let info = proxy
        .client(version)
        .login("Mallory")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(info.state, ConnectionState::Login, "{info:?}");
    assert_eq!(info.text, "No entry for Mallory", "{info:?}");
    let login = recorder
        .wait_for(|e| e.kind == EventKind::Login && named(e, "Mallory"), T)
        .await
        .unwrap();
    assert_eq!(
        login.detail["result"],
        json!({ "denied": "No entry for Mallory" })
    );
    assert_eq!(recorder.count(EventKind::PostLogin), 0);
    assert_eq!(recorder.count(EventKind::Disconnect), 0);
    assert_eq!(proxy.connection_count(), 0);
}

async fn offline_login_denied_ends_in_login(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(gatekeeper())
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    assert_login_denied(&proxy, &recorder, version).await;
    let session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), "Steve");

    session.quit().await;
    conn.closed(T).await.unwrap();
    proxy.shutdown().await.unwrap();
}

version_matrix!(offline_login_denied_ends_in_login; p47 = 47, p764 = 764, p774 = 774);

async fn client_only_login_denied_ends_in_login(version: ProtocolVersion) {
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("lobby").backend(backend.addr()))
        .session_server(&sessions)
        .plugin(gatekeeper())
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    assert_login_denied(&proxy, &recorder, version).await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(client_only_login_denied_ends_in_login; p47 = 47, p764 = 764, p774 = 774);

async fn denied_initial_connect_ends_with_one_disconnect(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let denier = ScriptedPlugin::new("denier")
        .on::<ServerPreConnectEvent>(EventPriority::NORMAL, |event| {
            event.deny(Component::text("Closed for maintenance"))
        });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(denier)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let info = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(info.text, "Closed for maintenance", "{info:?}");

    let disconnect = assert_single_disconnect_after_post_login(&proxy, &recorder, "Steve").await;
    assert_eq!(disconnect.detail["cause"], json!("kicked"));
    assert_eq!(disconnect.detail["reason"], json!("Closed for maintenance"));
    assert_eq!(disconnect.detail["last_server"], json!(null));
    proxy.shutdown().await.unwrap();
}

version_matrix!(denied_initial_connect_ends_with_one_disconnect; p47 = 47, p764 = 764, p774 = 774);

async fn unreachable_initial_backend_ends_with_one_disconnect(version: ProtocolVersion) {
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").unreachable())
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let info = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(info.state, ConnectionState::Login, "{info:?}");

    let disconnect = assert_single_disconnect_after_post_login(&proxy, &recorder, "Steve").await;
    assert_eq!(disconnect.detail["cause"], json!("error"));
    proxy.shutdown().await.unwrap();
}

version_matrix!(unreachable_initial_backend_ends_with_one_disconnect; p47 = 47, p764 = 764, p774 = 774);

async fn assert_single_disconnect_after_post_login(
    proxy: &TestProxy,
    recorder: &Recorder,
    username: &str,
) -> Recorded {
    let disconnect = recorder
        .wait_for(|e| e.kind == EventKind::Disconnect && named(e, username), T)
        .await
        .unwrap();
    proxy.wait_for_connection_count(0, T).await.unwrap();
    let post_logins = recorder.of(EventKind::PostLogin);
    assert_eq!(post_logins.len(), 1, "{post_logins:?}");
    assert_eq!(recorder.count(EventKind::Disconnect), 1);
    assert!(post_logins[0].seq < disconnect.seq);
    assert_eq!(post_logins[0].player, disconnect.player);
    disconnect
}

async fn last_server_follows_a_switch(version: ProtocolVersion) {
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("a")
                .backend(backend_a.addr())
                .network("main"),
        )
        .server(
            ServerSpec::offline("b")
                .backend(backend_b.addr())
                .network("main"),
        )
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let conn_a = backend_a.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    player.switch_server(ServerId::new("b")).await.unwrap();
    session.expect_join(T).await.unwrap();
    let conn_b = backend_b.next_connection(T).await.unwrap();
    conn_a.closed(T).await.unwrap();

    session.quit().await;
    let disconnect = recorder
        .wait_for(|e| e.kind == EventKind::Disconnect && named(e, "Steve"), T)
        .await
        .unwrap();
    assert_eq!(disconnect.detail["last_server"], json!("b"));
    assert_eq!(disconnect.detail["cause"], json!("client_quit"));
    conn_b.closed(T).await.unwrap();

    proxy.shutdown().await.unwrap();
}

version_matrix!(last_server_follows_a_switch; p47 = 47, p764 = 764, p774 = 774);

type Sighting = (Option<(String, Option<ServerId>)>, Option<ServerId>);

async fn post_login_sees_a_registered_player(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (sightings, mut seen) = mpsc::unbounded_channel::<Sighting>();
    let probe = ScriptedPlugin::new("probe").on_enable(move |ctx| {
        let registry = ctx.player_registry_handle();
        let sightings = sightings.clone();
        ctx.event_bus()
            .subscribe::<PostLoginEvent, _>(EventPriority::NORMAL, move |event| {
                let found = registry
                    .get_player_by_id(event.player_id())
                    .map(|p| (p.profile().username.clone(), p.current_server()));
                let _ = sightings.send((found, event.player.current_server()));
            });
    });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(probe)
        .start()
        .await
        .unwrap();

    let session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let (found, own_server) = tokio::time::timeout(T, seen.recv())
        .await
        .expect("PostLogin never reached the probe")
        .unwrap();

    assert_eq!(found, Some(("Steve".to_string(), None)));
    assert_eq!(own_server, None);

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(post_login_sees_a_registered_player; p47 = 47, p764 = 764, p774 = 774);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hung_disconnect_listener_is_cut_off_at_the_deadline() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let stuck = ScriptedPlugin::new("stuck")
        .on_async::<DisconnectEvent>(EventPriority::NORMAL, |_| Box::pin(std::future::pending()));
    let builder = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(stuck);
    let proxy = patch_section(
        builder,
        "events",
        vec![
            ("handler_timeout", Value::String("60s".into())),
            ("disconnect_deadline", Value::String("300ms".into())),
        ],
    )
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
    let conn = backend.next_connection(T).await.unwrap();
    proxy.wait_for_connection_count(1, T).await.unwrap();

    let quit_at = Instant::now();
    session.quit().await;
    conn.closed(T).await.unwrap();
    proxy.wait_for_connection_count(0, T).await.unwrap();
    let elapsed = quit_at.elapsed();

    assert!(elapsed >= Duration::from_millis(300), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panicking_post_login_listener_does_not_stop_the_join() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let panicky = ScriptedPlugin::new("panicky")
        .on::<PostLoginEvent>(EventPriority::NORMAL, |_| panic!("post login boom"));
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(panicky)
        .start()
        .await
        .unwrap();
    let mut diagnostics = proxy.bus().diagnostics();

    let session = proxy
        .client(ProtocolVersion(CURRENT))
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();

    let conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), "Steve");
    let diagnostic = tokio::time::timeout(T, diagnostics.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&*diagnostic.owner, "panicky");
    assert_eq!(diagnostic.event, "PostLoginEvent");
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Panicked {
            message: "post login boom".to_string()
        }
    );

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

async fn a_second_login_replaces_the_first(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut first = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let first_conn = backend.next_connection(T).await.unwrap();
    let first_id = recorder
        .wait_for(|e| e.kind == EventKind::PostLogin, T)
        .await
        .unwrap()
        .player;

    let second = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();

    let info = first.expect_disconnect(T).await.unwrap();
    assert_eq!(info.text, "You logged in from another location", "{info:?}");
    first_conn.closed(T).await.unwrap();
    let _second_conn = backend.next_connection(T).await.unwrap();

    let post_logins = recorder.of(EventKind::PostLogin);
    assert_eq!(post_logins.len(), 2, "{post_logins:?}");
    let disconnects = recorder.of(EventKind::Disconnect);
    assert_eq!(disconnects.len(), 1, "{disconnects:?}");
    assert_eq!(disconnects[0].player, first_id);
    assert_eq!(disconnects[0].detail["cause"], json!("kicked"));
    assert_eq!(
        disconnects[0].detail["reason"],
        json!("You logged in from another location")
    );
    assert!(disconnects[0].seq < post_logins[1].seq);
    let online = proxy.wait_for_player("Steve", T).await.unwrap();
    assert_eq!(Some(online.id()), post_logins[1].player);
    assert_eq!(proxy.connection_count(), 1);

    second.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(a_second_login_replaces_the_first; p47 = 47, p764 = 764, p774 = 774);

async fn a_kick_during_post_login_ends_in_login(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let bouncer =
        ScriptedPlugin::new("bouncer").on_async::<PostLoginEvent>(EventPriority::NORMAL, |event| {
            Box::pin(async move {
                event.player.disconnect(Component::text("Not today")).await;
            })
        });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(bouncer)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let info = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(info.state, ConnectionState::Login, "{info:?}");
    assert_eq!(info.text, "Not today", "{info:?}");
    let disconnect = assert_single_disconnect_after_post_login(&proxy, &recorder, "Steve").await;
    assert_eq!(disconnect.detail["cause"], json!("kicked"));
    assert_eq!(disconnect.detail["reason"], json!("Not today"));
    assert_eq!(recorder.count(EventKind::PlayerChooseInitialServer), 0);

    proxy.shutdown().await.unwrap();
}

version_matrix!(a_kick_during_post_login_ends_in_login; p47 = 47, p764 = 764, p774 = 774);

async fn a_post_login_message_arrives_after_join(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let greeter =
        ScriptedPlugin::new("greeter").on::<PostLoginEvent>(EventPriority::NORMAL, |event| {
            event
                .player
                .send_message(Component::text("Welcome aboard"))
                .unwrap();
        });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(greeter)
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();

    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "Welcome aboard"
    );
    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(a_post_login_message_arrives_after_join; p47 = 47, p764 = 764, p774 = 774);

const FORWARDED_SOURCE: &str = "203.0.113.7:51234";

async fn real_ip_comes_from_the_proxy_protocol_header(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .patch_config(|table| {
            table.insert("receive_proxy_protocol".into(), Value::Boolean(true));
        })
        .start()
        .await
        .unwrap();
    let source: SocketAddr = FORWARDED_SOURCE.parse().unwrap();

    let session = proxy
        .client(version)
        .proxy_protocol(source)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let pre_login = recorder
        .wait_for(|e| e.kind == EventKind::PreLogin && named(e, "Steve"), T)
        .await
        .unwrap();
    assert_eq!(pre_login.detail["remote_addr"], json!(FORWARDED_SOURCE));
    let post_login = recorder
        .wait_for(|e| e.kind == EventKind::PostLogin && named(e, "Steve"), T)
        .await
        .unwrap();
    assert_eq!(post_login.detail["remote_addr"], json!(FORWARDED_SOURCE));
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    assert_eq!(player.remote_addr(), source);

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(real_ip_comes_from_the_proxy_protocol_header; p47 = 47, p764 = 764, p774 = 774);

const CLAIMED: Uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);

async fn post_login_uuid(proxy: &TestProxy, recorder: &Recorder, version: ProtocolVersion) -> Uuid {
    let session = proxy
        .client(version)
        .claimed_uuid(CLAIMED)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let post_login = recorder
        .wait_for(|e| e.kind == EventKind::PostLogin && named(e, "Steve"), T)
        .await
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    let uuid: Uuid = post_login.detail["profile"]["uuid"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(player.profile().uuid, uuid);
    session.quit().await;
    uuid
}

async fn offline_profile_uses_the_offline_uuid(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    assert_eq!(
        post_login_uuid(&proxy, &recorder, version).await,
        offline_uuid("Steve")
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(offline_profile_uses_the_offline_uuid; p47 = 47, p764 = 764, p774 = 774);

async fn the_client_policy_trusts_a_claimed_uuid(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let builder = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin());
    let proxy = patch_section(
        builder,
        "auth",
        vec![("offline_uuid", Value::String("client".into()))],
    )
    .start()
    .await
    .unwrap();

    let expected = if version.no_less_than(ProtocolVersion::V1_19_1) {
        CLAIMED
    } else {
        offline_uuid("Steve")
    };
    assert_eq!(post_login_uuid(&proxy, &recorder, version).await, expected);

    proxy.shutdown().await.unwrap();
}

version_matrix!(the_client_policy_trusts_a_claimed_uuid; p47 = 47, p774 = 774);

async fn passthrough_profile_uses_the_offline_uuid(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let session = proxy
        .client(version)
        .claimed_uuid(CLAIMED)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    assert_eq!(player.profile().uuid, offline_uuid("Steve"));

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(passthrough_profile_uses_the_offline_uuid; p47 = 47, p774 = 774);

async fn passthrough_post_login_precedes_disconnect(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .patch_config(|table| {
            table.insert("receive_proxy_protocol".into(), Value::Boolean(true));
        })
        .start()
        .await
        .unwrap();

    let session = proxy
        .client(version)
        .proxy_protocol(FORWARDED_SOURCE.parse().unwrap())
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    session.quit().await;
    conn.closed(T).await.unwrap();
    conn.close().await;

    let disconnect = assert_single_disconnect_after_post_login(&proxy, &recorder, "Steve").await;
    assert_eq!(disconnect.detail["last_server"], json!("lobby"));
    let post_login = &recorder.of(EventKind::PostLogin)[0];
    assert_eq!(post_login.detail["remote_addr"], json!(FORWARDED_SOURCE));
    assert_eq!(post_login.detail["current_server"], json!(null));

    proxy.shutdown().await.unwrap();
}

version_matrix!(passthrough_post_login_precedes_disconnect; p47 = 47, p774 = 774);

const REWRITTEN: Uuid = Uuid::from_u128(0x00ff_00ff_00ff_00ff_00ff_00ff_00ff_00ff);

fn rewriter() -> ScriptedPlugin {
    ScriptedPlugin::new("rewriter").on::<GameProfileRequestEvent>(EventPriority::NORMAL, |event| {
        event.profile.uuid = REWRITTEN;
        event.profile.username = "Renamed".into();
    })
}

async fn assert_rewritten_profile(
    proxy: &TestProxy,
    recorder: &Recorder,
    backend: &FakeBackend,
    version: ProtocolVersion,
    username: &str,
) {
    let session = proxy
        .client(version)
        .login(username)
        .await
        .unwrap()
        .joined()
        .unwrap();

    assert_eq!(session.uuid(), Some(REWRITTEN));
    assert_eq!(session.profile_name(), Some("Renamed"));
    let conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), "Renamed");
    let post_login = recorder
        .wait_for(|e| e.kind == EventKind::PostLogin, T)
        .await
        .unwrap();
    assert_eq!(
        post_login.detail["profile"]["uuid"],
        json!(REWRITTEN.to_string())
    );
    assert_eq!(post_login.detail["profile"]["username"], json!("Renamed"));
    let request = &recorder.of(EventKind::GameProfileRequest)[0];
    assert_eq!(request.detail["original"]["username"], json!(username));
    assert_eq!(
        proxy
            .wait_for_player("Renamed", T)
            .await
            .unwrap()
            .profile()
            .uuid,
        REWRITTEN
    );

    session.quit().await;
    conn.closed(T).await.unwrap();
}

async fn client_only_profile_can_be_rewritten(version: ProtocolVersion) {
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("lobby").backend(backend.addr()))
        .session_server(&sessions)
        .plugin(rewriter())
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    assert_rewritten_profile(&proxy, &recorder, &backend, version, "Alex").await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(client_only_profile_can_be_rewritten;
    p47 = 47, p340 = 340, p763 = 763, p764 = 764, p766 = 766, p774 = 774);

async fn offline_profile_can_be_rewritten(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(rewriter())
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    assert_rewritten_profile(&proxy, &recorder, &backend, version, "Steve").await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(offline_profile_can_be_rewritten; p47 = 47, p764 = 764, p774 = 774);
