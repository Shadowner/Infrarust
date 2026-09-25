#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use infrarust_api::event::EventPriority;
use infrarust_api::events::lifecycle::{DisconnectEvent, LoginEvent};
use infrarust_api::events::proxy::{ConfigReloadEvent, ProxyShutdownEvent};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_protocol::codec::VarInt;
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::login::CLoginDisconnect;
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, EventKind, FakeBackend, FramedConn, ProtocolVersion,
    Recorded, Recorder, ScriptedPlugin, ServerSpec, TestProxy, text, version_matrix, wire,
};
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::Notify;
use toml::{Table, Value};

const T: Duration = DEFAULT_TIMEOUT;
const SHUTDOWN_MESSAGE: &str = "Proxy is shutting down";

type Snapshot = Arc<Mutex<Option<Vec<EventKind>>>>;

fn disable_probe(recorder: &Recorder) -> (ScriptedPlugin, Snapshot) {
    let snapshot = Snapshot::default();
    let slot = Arc::clone(&snapshot);
    let recorder = recorder.clone();
    let probe = ScriptedPlugin::new("disable_probe").on_disable(move || {
        *slot.lock().unwrap() = Some(recorder.kinds());
    });
    (probe, snapshot)
}

fn position(events: &[Recorded], pick: impl Fn(&Recorded) -> bool) -> u64 {
    events
        .iter()
        .find(|e| pick(e))
        .map(|e| e.seq)
        .unwrap_or_else(|| panic!("event not recorded in {events:#?}"))
}

fn disconnect_of<'a>(events: &'a [Recorded], username: &str) -> &'a Recorded {
    let found: Vec<&Recorded> = events
        .iter()
        .filter(|e| e.kind == EventKind::Disconnect && e.username.as_deref() == Some(username))
        .collect();
    assert_eq!(found.len(), 1, "{events:#?}");
    found[0]
}

fn disabled_after(snapshot: &Snapshot) -> Vec<EventKind> {
    snapshot
        .lock()
        .unwrap()
        .clone()
        .expect("on_disable must run during shutdown")
}

async fn a_connected_player_is_kicked_before_plugins_stop(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let (probe, snapshot) = disable_probe(&recorder);
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .plugin(probe)
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

    let (stopped, kicked) = tokio::join!(proxy.shutdown(), session.expect_disconnect(T));

    stopped.unwrap();
    let info = kicked.unwrap();
    assert_eq!(info.state, ConnectionState::Play, "{info:?}");
    assert_eq!(info.text, SHUTDOWN_MESSAGE, "{info:?}");
    let events = recorder.events();
    let disconnect = disconnect_of(&events, "Steve");
    assert_eq!(disconnect.detail["cause"], json!("shutdown"));
    assert_eq!(disconnect.detail["last_server"], json!("lobby"));
    let shutdown = position(&events, |e| e.kind == EventKind::ProxyShutdown);
    assert!(disconnect.seq < shutdown, "{events:#?}");
    let seen = disabled_after(&snapshot);
    assert!(seen.contains(&EventKind::Disconnect), "{seen:?}");
    assert!(seen.contains(&EventKind::ProxyShutdown), "{seen:?}");
}

version_matrix!(a_connected_player_is_kicked_before_plugins_stop; p47 = 47, p764 = 764, p774 = 774);

async fn a_passthrough_player_disconnects_before_proxy_shutdown(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("pipe").backend(backend.addr()))
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
    let conn = backend.next_connection(T).await.unwrap();
    proxy.wait_for_player("Steve", T).await.unwrap();

    proxy.shutdown().await.unwrap();

    conn.closed(T).await.unwrap();
    let events = recorder.events();
    let disconnect = disconnect_of(&events, "Steve");
    assert_eq!(disconnect.detail["cause"], json!("shutdown"));
    let shutdown = position(&events, |e| e.kind == EventKind::ProxyShutdown);
    assert!(disconnect.seq < shutdown, "{events:#?}");
}

version_matrix!(a_passthrough_player_disconnects_before_proxy_shutdown; p47 = 47, p774 = 774);

#[tokio::test]
async fn events_posted_during_shutdown_reach_plugins_before_they_are_disabled() {
    let recorder = Recorder::new();
    let (probe, snapshot) = disable_probe(&recorder);
    let bus: Arc<OnceLock<Arc<EventBusImpl>>> = Arc::default();
    let poster = {
        let bus = Arc::clone(&bus);
        ScriptedPlugin::new("poster").on::<ProxyShutdownEvent>(EventPriority::NORMAL, move |_| {
            bus.get()
                .expect("the bus is known once the proxy runs")
                .post(ConfigReloadEvent::new(
                    "file",
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ));
        })
    };
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").unreachable())
        .plugin(recorder.plugin())
        .plugin(poster)
        .plugin(probe)
        .start()
        .await
        .unwrap();
    bus.set(Arc::clone(proxy.bus())).ok();

    proxy.shutdown().await.unwrap();

    let events = recorder.events();
    let shutdown = position(&events, |e| e.kind == EventKind::ProxyShutdown);
    let reload = position(&events, |e| e.kind == EventKind::ConfigReload);
    assert!(shutdown < reload, "{events:#?}");
    let seen: Vec<EventKind> = disabled_after(&snapshot)
        .into_iter()
        .filter(|kind| matches!(kind, EventKind::ProxyShutdown | EventKind::ConfigReload))
        .collect();
    assert_eq!(seen, [EventKind::ProxyShutdown, EventKind::ConfigReload]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hung_disconnect_listener_holds_shutdown_only_until_the_deadline() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let (probe, snapshot) = disable_probe(&recorder);
    let hang = ScriptedPlugin::new("hang")
        .on_async::<DisconnectEvent>(EventPriority::NORMAL, |_| Box::pin(std::future::pending()));
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(hang)
        .plugin(recorder.plugin())
        .plugin(probe)
        .drain_timeout(Duration::from_secs(60))
        .patch_config(|table| {
            table.insert(
                "events".into(),
                Value::Table(Table::from_iter([(
                    "disconnect_deadline".into(),
                    Value::String("300ms".into()),
                )])),
            );
        })
        .start()
        .await
        .unwrap();
    let _session = proxy
        .client(ProtocolVersion(774))
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();
    proxy.wait_for_player("Steve", T).await.unwrap();
    let registry = Arc::clone(&proxy.services().connection_registry);

    tokio::time::timeout(T, proxy.shutdown())
        .await
        .expect("a hung Disconnect listener must not hold shutdown past disconnect_deadline")
        .unwrap();

    assert_eq!(registry.count(), 0);
    assert_eq!(recorder.count(EventKind::ProxyShutdown), 1);
    assert!(disabled_after(&snapshot).contains(&EventKind::ProxyShutdown));
}

async fn a_login_in_progress_gets_no_post_login_after_shutdown_began(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let gate = {
        let entered = Arc::clone(&entered);
        let release_on_login = Arc::clone(&release);
        let release_on_disconnect = Arc::clone(&release);
        ScriptedPlugin::new("gate")
            .on_async::<LoginEvent>(EventPriority::NORMAL, move |event| {
                let hold = event.profile().username == "Late";
                let entered = Arc::clone(&entered);
                let release = Arc::clone(&release_on_login);
                Box::pin(async move {
                    if hold {
                        entered.notify_one();
                        release.notified().await;
                    }
                })
            })
            .on::<DisconnectEvent>(EventPriority::NORMAL, move |event| {
                if event.username() == "Steve" {
                    release_on_disconnect.notify_one();
                }
            })
    };
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(gate)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    let mut steve = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();
    proxy.wait_for_player("Steve", T).await.unwrap();
    let late_client = proxy.client(version);
    let late = tokio::spawn(async move { late_client.login("Late").await });
    tokio::time::timeout(T, entered.notified())
        .await
        .expect("Late must reach the Login event");

    let (stopped, kicked) = tokio::join!(proxy.shutdown(), steve.expect_disconnect(T));

    stopped.unwrap();
    assert_eq!(kicked.unwrap().text, SHUTDOWN_MESSAGE);
    let info = tokio::time::timeout(T, late)
        .await
        .expect("Late's login must end")
        .unwrap()
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(info.state, ConnectionState::Login, "{info:?}");
    assert_eq!(info.text, SHUTDOWN_MESSAGE, "{info:?}");
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
    let events = recorder.events();
    let disconnect = disconnect_of(&events, "Steve");
    assert_eq!(disconnect.detail["cause"], json!("shutdown"));
}

version_matrix!(a_login_in_progress_gets_no_post_login_after_shutdown_began; p47 = 47, p764 = 764, p774 = 774);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connections_that_never_log_in_do_not_hold_shutdown() {
    let version = ProtocolVersion(774);
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .drain_timeout(Duration::from_secs(60))
        .start()
        .await
        .unwrap();
    let mut silent = TcpStream::connect(proxy.addr()).await.unwrap();
    let mut handshake_only = FramedConn::connect(proxy.addr()).await.unwrap();
    let handshake = SHandshake {
        protocol_version: VarInt(version.0),
        server_address: proxy.domain("lobby").unwrap().to_string(),
        server_port: proxy.addr().port(),
        next_state: ConnectionState::Login,
    };
    handshake_only
        .write_frame(&wire::encode(&handshake, version).unwrap())
        .await
        .unwrap();
    let server = Arc::clone(proxy.server());
    tokio::time::timeout(T, async {
        while server.active_connections() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the proxy must accept both connections");

    tokio::time::timeout(T, proxy.shutdown())
        .await
        .expect("idle connections must not hold shutdown until their read timeouts")
        .unwrap();

    assert_eq!(server.active_connections(), 0);
    let mut rest = Vec::new();
    silent.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty());
    if let Ok(Some(frame)) = handshake_only.read_frame().await {
        let kick = wire::decode::<CLoginDisconnect>(&frame, version).unwrap();
        assert_eq!(
            text::DisconnectInfo::from_json_string(ConnectionState::Login, &kick.reason).text,
            SHUTDOWN_MESSAGE
        );
    }
}
