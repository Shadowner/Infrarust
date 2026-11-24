#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use infrarust_api::event::{BoxFuture, EventPriority};
use infrarust_api::events::lifecycle::LoginEvent;
use infrarust_api::filter::{FilterMetadata, FilterVerdict, TransportContext, TransportFilter};
use infrarust_config::ProxyMode;
use infrarust_core::player::{RESTART_REASON, SHUTDOWN_REASON, ShutdownKind};
use infrarust_protocol::packets::play::chat::SChatMessage;
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, EventKind, FakeBackend, ProtocolVersion, Recorder,
    ScriptedPlugin, ServerSpec, TestProxy, version_matrix,
};
use tokio::sync::Notify;

const T: Duration = DEFAULT_TIMEOUT;

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let waited = tokio::time::timeout(T, async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "timed out waiting for {what}");
}

async fn a_forwarding_session_keeps_relaying_through_a_restart_close(mode: ProxyMode) {
    let version = ProtocolVersion(777);
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::new("lobby", mode).backend(backend.addr()))
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
    let mut conn = backend.next_connection(T).await.unwrap();
    proxy.wait_for_player("Steve", T).await.unwrap();
    let server = Arc::clone(proxy.server());
    let relays = Arc::clone(server.relays());
    assert_eq!(relays.active(), 1);
    assert_eq!(relays.in_flight(), 0);

    server.close_sessions(ShutdownKind::Restart);

    assert_eq!(server.drain_pipeline(T).await, 0);
    session.chat("after the cut").await.unwrap();
    assert_eq!(
        conn.expect::<SChatMessage>(T).await.unwrap().message,
        "after the cut"
    );
    conn.send_system_message_json(r#"{"text":"still here"}"#)
        .await
        .unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "still here");
    assert_eq!(recorder.count(EventKind::Disconnect), 0);
    assert_eq!(proxy.connection_count(), 1);
    assert_eq!(relays.active(), 1);

    proxy.shutdown().await.unwrap();

    conn.closed(T).await.unwrap();
    let disconnect = recorder
        .wait_for(
            |e| e.kind == EventKind::Disconnect && e.is_named("Steve"),
            T,
        )
        .await
        .unwrap();
    assert_eq!(disconnect.cause(), "shutdown");
    assert_eq!(relays.active(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_passthrough_session_keeps_relaying_through_a_restart_close() {
    a_forwarding_session_keeps_relaying_through_a_restart_close(ProxyMode::Passthrough).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_zero_copy_session_keeps_relaying_through_a_restart_close() {
    a_forwarding_session_keeps_relaying_through_a_restart_close(ProxyMode::ZeroCopy).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_only_session_keeps_relaying_through_a_restart_close() {
    a_forwarding_session_keeps_relaying_through_a_restart_close(ProxyMode::ServerOnly).await;
}

async fn an_offline_session_is_told_the_proxy_restarts(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
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
    let _conn = backend.next_connection(T).await.unwrap();
    proxy.wait_for_player("Steve", T).await.unwrap();
    let server = Arc::clone(proxy.server());
    assert_eq!(server.relays().in_flight(), 0);

    server.close_sessions(ShutdownKind::Restart);

    let info = session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.state, ConnectionState::Play, "{info:?}");
    assert_eq!(info.text, RESTART_REASON, "{info:?}");
    let disconnect = recorder
        .wait_for(
            |e| e.kind == EventKind::Disconnect && e.is_named("Steve"),
            T,
        )
        .await
        .unwrap();
    assert_eq!(disconnect.cause(), "shutdown");
    assert_eq!(server.drain_pipeline(T).await, 0);
    assert_eq!(server.relays().active(), 0);
    proxy.wait_for_connection_count(0, T).await.unwrap();
    proxy.shutdown().await.unwrap();
}

version_matrix!(an_offline_session_is_told_the_proxy_restarts; p47 = 47, p764 = 764, p777 = 777);

fn held_login(username: &'static str) -> (ScriptedPlugin, Arc<Notify>, Arc<Notify>) {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let gate = {
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        ScriptedPlugin::new("gate").on_async::<LoginEvent>(EventPriority::NORMAL, move |event| {
            let hold = event.profile().username == username;
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            Box::pin(async move {
                if hold {
                    entered.notify_one();
                    release.notified().await;
                }
            })
        })
    };
    (gate, entered, release)
}

async fn a_forwarded_login_in_progress_is_cut_by_a_restart_close(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let (gate, entered, release) = held_login("Late");
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
        .plugin(gate)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    let late_client = proxy.client(version);
    let late = tokio::spawn(async move { late_client.login("Late").await });
    tokio::time::timeout(T, entered.notified())
        .await
        .expect("Late must reach the Login event");
    let server = Arc::clone(proxy.server());
    let relays = Arc::clone(server.relays());
    assert_eq!(relays.in_flight(), 1);
    assert_eq!(relays.linked_logins(), 1);

    server.close_sessions(ShutdownKind::Restart);
    release.notify_one();

    let info = tokio::time::timeout(T, late)
        .await
        .expect("Late's login must end")
        .unwrap()
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(info.state, ConnectionState::Login, "{info:?}");
    assert_eq!(info.text, RESTART_REASON, "{info:?}");
    assert_eq!(server.drain_pipeline(T).await, 0);
    assert_eq!(relays.in_flight(), 0);
    assert_eq!(relays.active(), 0);
    assert_eq!(relays.linked_logins(), 0);
    assert_eq!(backend.accepted_connections(), 0);
    let late_events: Vec<EventKind> = recorder
        .for_username("Late")
        .iter()
        .map(|e| e.kind)
        .collect();
    assert!(
        !late_events.contains(&EventKind::PostLogin),
        "{late_events:?}"
    );
    assert!(
        !late_events.contains(&EventKind::Disconnect),
        "{late_events:?}"
    );
    proxy.shutdown().await.unwrap();
}

version_matrix!(a_forwarded_login_in_progress_is_cut_by_a_restart_close; p47 = 47, p777 = 777);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_close_still_ends_every_session() {
    let version = ProtocolVersion(777);
    let pipe = FakeBackend::builder().spawn().await.unwrap();
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("pipe").backend(pipe.addr()))
        .server(ServerSpec::offline("lobby").backend(lobby.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    let _forwarded = proxy
        .client_for("pipe", version)
        .unwrap()
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let pipe_conn = pipe.next_connection(T).await.unwrap();
    let mut intercepted = proxy
        .client_for("lobby", version)
        .unwrap()
        .login("Alex")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _lobby_conn = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Steve", T).await.unwrap();
    proxy.wait_for_player("Alex", T).await.unwrap();
    let server = Arc::clone(proxy.server());

    server.close_sessions(ShutdownKind::Stop);

    let info = intercepted.expect_disconnect(T).await.unwrap();
    assert_eq!(info.text, SHUTDOWN_REASON, "{info:?}");
    pipe_conn.closed(T).await.unwrap();
    proxy.wait_for_connection_count(0, T).await.unwrap();
    for username in ["Steve", "Alex"] {
        let disconnect = recorder
            .wait_for(
                |e| e.kind == EventKind::Disconnect && e.is_named(username),
                T,
            )
            .await
            .unwrap();
        assert_eq!(disconnect.cause(), "shutdown", "{username}");
    }
    assert_eq!(server.relays().active(), 0);
    proxy.shutdown().await.unwrap();
}

struct CloseCounter(Arc<AtomicUsize>);

impl TransportFilter for CloseCounter {
    fn metadata(&self) -> FilterMetadata {
        FilterMetadata::new("close_counter")
    }

    fn on_accept<'a>(&'a self, _ctx: &'a mut TransportContext) -> BoxFuture<'a, FilterVerdict> {
        Box::pin(async { FilterVerdict::Continue })
    }

    fn on_close(&self, _ctx: &TransportContext) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn counting_closes() -> (ScriptedPlugin, Arc<AtomicUsize>) {
    let closes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&closes);
    let plugin = ScriptedPlugin::new("closes").on_enable(move |ctx| {
        ctx.transport_filters()
            .expect("trusted plugins hold the transport-filter capability")
            .register(Box::new(CloseCounter(Arc::clone(&counter))))
            .expect("the filter id is free");
    });
    (plugin, closes)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the forwarder freeze"]
async fn a_frozen_passthrough_session_is_collected_for_the_successor() {
    let version = ProtocolVersion(777);
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let (closes_plugin, closes) = counting_closes();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
        .plugin(closes_plugin)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    let _session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    let server = Arc::clone(proxy.server());
    let relays = Arc::clone(server.relays());
    assert_eq!(relays.active(), 1);

    relays.begin_freeze();

    assert_eq!(relays.drained(tokio::time::Instant::now() + T).await, 0);
    wait_until("the frozen connection task to end", || {
        server.active_connections() == 0
    })
    .await;
    let frozen = relays.take_frozen();
    assert_eq!(frozen.len(), 1);
    let meta = &frozen[0].meta;
    assert_eq!(meta.player_id, player.id().as_u64());
    assert_eq!(meta.profile.username, "Steve");
    assert_eq!(meta.profile.uuid, player.profile().uuid);
    assert_eq!(meta.protocol_version, version.0);
    assert_eq!(meta.mode, ProxyMode::Passthrough);
    assert_eq!(meta.server, "lobby");
    assert_eq!(
        meta.client_addr.to_string(),
        recorder.of(EventKind::PreLogin)[0].remote_addr()
    );
    assert_eq!(
        meta.connected_address.as_deref(),
        Some(backend.addr().to_string().as_str())
    );
    assert_eq!(meta.virtual_host.as_deref(), Some("lobby.test"));
    assert!(meta.connected_at_ms > 0);
    assert_eq!(recorder.count(EventKind::Disconnect), 0);
    assert_eq!(proxy.connection_count(), 0);
    assert_eq!(relays.active(), 0);
    assert_eq!(closes.load(Ordering::SeqCst), 0);
    drop(frozen);
    proxy.shutdown().await.unwrap();
    assert_eq!(recorder.count(EventKind::Disconnect), 0);
}
