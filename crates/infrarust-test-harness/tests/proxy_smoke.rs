#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::event::EventPriority;
use infrarust_api::events::chat::ChatMessageEvent;
use infrarust_api::events::lifecycle::PreLoginEvent;
use infrarust_api::types::{Component, ServerId};
use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_protocol::packets::play::chat::SChatMessage;
use infrarust_test_harness::versions::CURRENT;
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, EventKind, FakeBackend, FakeSessionServer, LoginOutcome,
    ProtocolVersion, Recorded, Recorder, ScriptedPlugin, ServerSpec, TestProxy, version_matrix,
};
use serde_json::json;

const T: Duration = DEFAULT_TIMEOUT;

#[test]
fn ad_hoc_matrices_end_at_the_current_protocol() {
    assert_eq!(CURRENT, 774);
}

async fn offline_login(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
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
    let mut conn = backend.next_connection(T).await.unwrap();

    assert_eq!(conn.username(), "Steve");
    let handshake = conn.handshake();
    assert_eq!(handshake.protocol_version, version.0);
    assert_eq!(handshake.server_address, "lobby.test");
    assert_eq!(handshake.next_state, 2);
    assert_eq!(session.uuid(), Some(offline_uuid("Steve")));

    session.chat("hi").await.unwrap();
    assert_eq!(conn.expect::<SChatMessage>(T).await.unwrap().message, "hi");

    conn.send_system_message_json(r#"{"text":"Welcome","extra":[{"text":" back"}]}"#)
        .await
        .unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "Welcome back");

    session.quit().await;
    conn.closed(T).await.unwrap();
    proxy.wait_for_connection_count(0, T).await.unwrap();
    proxy.shutdown().await.unwrap();
}

version_matrix!(LIFECYCLE, offline_login);

async fn client_only_login(version: ProtocolVersion) {
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("lobby").backend(backend.addr()))
        .session_server(&sessions)
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login("Alex")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let hash = session
        .server_hash()
        .expect("a client_only server must request encryption")
        .to_string();
    assert_eq!(sessions.calls(), vec![("Alex".to_string(), hash)]);
    assert_eq!(session.uuid(), Some(offline_uuid("Alex")));

    let mut conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), "Alex");

    session.chat("sealed hello").await.unwrap();
    assert_eq!(
        conn.expect::<SChatMessage>(T).await.unwrap().message,
        "sealed hello"
    );
    conn.send_system_message_json(r#"{"text":"sealed reply"}"#)
        .await
        .unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "sealed reply");

    proxy.shutdown().await.unwrap();
}

version_matrix!(client_only_login;
    p47 = 47, p340 = 340, p763 = 763, p764 = 764, p766 = 766, p774 = 774);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_only_rejected_by_the_session_server() {
    let sessions = FakeSessionServer::spawn().await.unwrap();
    sessions.reject("Mallory");
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("lobby").backend(backend.addr()))
        .session_server(&sessions)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let outcome = proxy
        .client(ProtocolVersion(CURRENT))
        .login("Mallory")
        .await;
    assert!(
        !matches!(outcome, Ok(LoginOutcome::Joined(_))),
        "{outcome:?}"
    );

    let call = sessions.wait_for_call("Mallory", T).await.unwrap();
    assert_eq!(call.username, "Mallory");
    recorder
        .wait_for(
            |e| e.kind == EventKind::OnlineAuthFailed && named(e, "Mallory"),
            T,
        )
        .await
        .unwrap();
    assert_eq!(recorder.count(EventKind::PostLogin), 0);
    assert_eq!(proxy.connection_count(), 0);

    proxy.shutdown().await.unwrap();
}

async fn status_through_proxy(version: ProtocolVersion) {
    let backend = FakeBackend::builder()
        .status(json!({
            "version": { "name": "Harness 1.0", "protocol": version.0 },
            "players": { "max": 42, "online": 7 },
            "description": { "text": "Relayed by Infrarust" },
            "enforcesSecureChat": true,
        }))
        .spawn()
        .await
        .unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let status = proxy.client(version).status().await.unwrap();

    assert_eq!(
        status.json["version"],
        json!({ "name": "Harness 1.0", "protocol": version.0 })
    );
    assert_eq!(status.json["players"]["max"], json!(42));
    assert_eq!(status.json["players"]["online"], json!(7));
    assert_eq!(
        status.json["description"],
        json!({ "text": "Relayed by Infrarust" })
    );
    assert_eq!(status.json["enforcesSecureChat"], json!(true));
    assert_eq!(backend.status_requests(), 1);

    proxy.shutdown().await.unwrap();
}

version_matrix!(LIFECYCLE, status_through_proxy);

async fn passthrough_login(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
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
    let mut conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), "Steve");
    assert_eq!(conn.handshake().server_address, "lobby.test");
    assert_eq!(conn.handshake().protocol_version, version.0);

    session.chat("through the pipe").await.unwrap();
    assert_eq!(
        conn.expect::<SChatMessage>(T).await.unwrap().message,
        "through the pipe"
    );
    conn.send_system_message_json(r#"{"text":"and back"}"#)
        .await
        .unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "and back");

    session.quit().await;
    conn.closed(T).await.unwrap();
    conn.close().await;
    proxy.wait_for_connection_count(0, T).await.unwrap();
    proxy.shutdown().await.unwrap();
}

version_matrix!(passthrough_login; p47 = 47, p764 = 764, p774 = 774);

async fn switch_between_backends(version: ProtocolVersion) {
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
    assert_eq!(conn_a.username(), "Steve");

    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    assert_eq!(player.current_server(), Some(ServerId::new("a")));
    player.switch_server(ServerId::new("b")).await.unwrap();

    session.expect_join(T).await.unwrap();
    let mut conn_b = backend_b.next_connection(T).await.unwrap();
    assert_eq!(conn_b.username(), "Steve");

    session.chat("hello b").await.unwrap();
    assert_eq!(
        conn_b.expect::<SChatMessage>(T).await.unwrap().message,
        "hello b"
    );
    conn_a.closed(T).await.unwrap();

    let switch = recorder
        .wait_for(
            |e| e.kind == EventKind::ServerPostConnect && e.detail["server"] == json!("b"),
            T,
        )
        .await
        .unwrap();
    assert_eq!(switch.player, Some(player.id()));
    assert_eq!(switch.username.as_deref(), Some("Steve"));
    assert_eq!(
        switch.detail,
        json!({ "server": "b", "previous_server": "a", "current_server": "b" })
    );
    assert_eq!(player.current_server(), Some(ServerId::new("b")));

    proxy.shutdown().await.unwrap();
}

version_matrix!(SWITCH, switch_between_backends);

fn named(event: &Recorded, username: &str) -> bool {
    event.username.as_deref() == Some(username)
}

async fn recorder_sees_basic_lifecycle(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
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
    let conn = backend.next_connection(T).await.unwrap();

    let post_login = recorder
        .wait_for(|e| e.kind == EventKind::PostLogin && named(e, "Steve"), T)
        .await
        .unwrap();
    let player = post_login.player.expect("PostLogin carries the player id");

    session.quit().await;
    let disconnect = recorder
        .wait_for(
            |e| e.kind == EventKind::Disconnect && e.player == Some(player),
            T,
        )
        .await
        .unwrap();
    conn.closed(T).await.unwrap();

    for kind in [
        EventKind::PreLogin,
        EventKind::PostLogin,
        EventKind::ServerPreConnect,
        EventKind::ServerConnected,
        EventKind::ServerPostConnect,
        EventKind::Disconnect,
    ] {
        recorder
            .wait_for(|e| e.kind == kind && named(e, "Steve"), T)
            .await
            .unwrap();
        let seen = recorder.filter(|e| e.kind == kind && named(e, "Steve"));
        assert_eq!(seen.len(), 1, "{kind} for Steve: {seen:?}");
        if kind != EventKind::PreLogin {
            assert_eq!(seen[0].player, Some(player), "{kind}");
        }
    }

    let pre_login = &recorder.of(EventKind::PreLogin)[0];
    assert_eq!(pre_login.detail["server_domain"], json!("lobby.test"));
    assert_eq!(pre_login.detail["protocol_version"], json!(version.0));
    assert_eq!(pre_login.detail["result"], json!("allowed"));
    let pre_connect = &recorder.of(EventKind::ServerPreConnect)[0];
    assert_eq!(pre_connect.detail["server"], json!("lobby"));
    assert_eq!(pre_connect.detail["cause"], json!("initial"));
    assert_eq!(pre_connect.detail["result"], json!("allowed"));
    let connected = &recorder.of(EventKind::ServerConnected)[0];
    assert_eq!(
        connected.detail,
        json!({ "server": "lobby", "previous_server": null, "current_server": null })
    );
    assert_eq!(disconnect.detail["last_server"], json!("lobby"));

    let order: Vec<EventKind> = recorder
        .for_username("Steve")
        .iter()
        .map(|e| e.kind)
        .collect();
    eprintln!("lifecycle order for protocol {}: {order:?}", version.0);

    proxy.shutdown().await.unwrap();
}

version_matrix!(recorder_sees_basic_lifecycle; p47 = 47, p340 = 340, p764 = 764, p774 = 774);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proxy_lifecycle_events() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let initialize = recorder
        .wait_for_kind(EventKind::ProxyInitialize, T)
        .await
        .unwrap();
    assert_eq!(recorder.kinds(), vec![EventKind::ProxyInitialize]);

    let session = proxy
        .client(ProtocolVersion(CURRENT))
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();
    let pre_login = recorder
        .wait_for_kind(EventKind::PreLogin, T)
        .await
        .unwrap();
    assert!(initialize.seq < pre_login.seq);

    session.quit().await;
    recorder
        .wait_for_kind(EventKind::Disconnect, T)
        .await
        .unwrap();
    assert_eq!(recorder.count(EventKind::ProxyShutdown), 0);

    proxy.shutdown().await.unwrap();

    assert_eq!(recorder.count(EventKind::ProxyInitialize), 1);
    assert_eq!(recorder.count(EventKind::ProxyShutdown), 1);
    assert_eq!(
        recorder.events().last().map(|e| e.kind),
        Some(EventKind::ProxyShutdown)
    );
}

async fn scripted_plugin_hooks(version: ProtocolVersion) {
    let enabled_as = Arc::new(Mutex::new(None::<String>));
    let enabled = Arc::clone(&enabled_as);
    let scripted = ScriptedPlugin::new("scripted")
        .on::<PreLoginEvent>(EventPriority::NORMAL, |event| {
            if event.profile.username == "Mallory" {
                event.deny(Component::text("No Mallory"));
            }
        })
        .on_async::<ChatMessageEvent>(EventPriority::NORMAL, |event| {
            Box::pin(async move {
                if event.message == "secret" {
                    event.deny(Component::text("hidden"));
                }
            })
        })
        .on_enable(move |ctx| {
            *enabled.lock().unwrap() = Some(ctx.plugin_id().to_string());
        });
    let recorder = Recorder::new();
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(scripted)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    assert_eq!(enabled_as.lock().unwrap().as_deref(), Some("scripted"));

    let denied = proxy
        .client(version)
        .login("Mallory")
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(denied.state, ConnectionState::Login);
    assert_eq!(denied.text, "No Mallory", "{denied:?}");
    let pre_login = recorder
        .wait_for(|e| e.kind == EventKind::PreLogin && named(e, "Mallory"), T)
        .await
        .unwrap();
    assert_eq!(
        pre_login.detail["result"],
        json!({ "denied": "No Mallory" })
    );

    let session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), "Steve");

    session.chat("secret").await.unwrap();
    session.chat("public").await.unwrap();
    assert_eq!(
        conn.expect::<SChatMessage>(T).await.unwrap().message,
        "public"
    );
    let chat = recorder
        .wait_for(
            |e| e.kind == EventKind::ChatMessage && e.detail["message"] == "secret",
            T,
        )
        .await
        .unwrap();
    assert_eq!(chat.username.as_deref(), Some("Steve"));
    assert_eq!(chat.detail["result"], json!({ "deny": "hidden" }));

    proxy.shutdown().await.unwrap();
}

version_matrix!(scripted_plugin_hooks; p47 = 47, p764 = 764, p774 = 774);
