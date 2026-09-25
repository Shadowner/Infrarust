#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use infrarust_api::event::EventPriority;
use infrarust_api::events::handshake::ConnectionHandshakeEvent;
use infrarust_api::services::ban_service::{BanEntry, BanSource, BanTarget};
use infrarust_api::types::{Component, ServerId};
use infrarust_core::ban::FileBanStorage;
use infrarust_core::ban::storage::BanStorage;
use infrarust_test_harness::legacy::LEGACY_PROTOCOL;
use infrarust_test_harness::versions::CURRENT;
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, EventKind, FakeBackend, FakeClient, HarnessError,
    ProtocolVersion, Recorded, Recorder, ScriptedPlugin, ServerSpec, TestProxy,
};
use serde_json::{Value, json};
use toml::Table;

const T: Duration = DEFAULT_TIMEOUT;
const VERSION: ProtocolVersion = ProtocolVersion(CURRENT);
const GATEKEEPER: &str = "gatekeeper";

fn from(ip: &str, port: u16) -> SocketAddr {
    SocketAddr::new(ip.parse().unwrap(), port)
}

fn receive_proxy_protocol(table: &mut Table) {
    table.insert("receive_proxy_protocol".into(), toml::Value::Boolean(true));
}

fn gatekeeper(
    decide: impl Fn(&mut ConnectionHandshakeEvent) + Send + Sync + 'static,
) -> ScriptedPlugin {
    ScriptedPlugin::new(GATEKEEPER).on::<ConnectionHandshakeEvent>(EventPriority::NORMAL, decide)
}

fn guarded(event: &mut ConnectionHandshakeEvent) {
    if event.server == Some(ServerId::new("guarded")) {
        match event.virtual_host.as_deref() {
            Some("guarded.test") => event.deny(Component::text("No bots allowed")),
            _ => event.drop_silently(),
        }
    }
}

async fn settle(proxy: &TestProxy) {
    tokio::time::timeout(T, proxy.bus().flush())
        .await
        .expect("the event queue never drained");
}

async fn ban_file(dir: &Path, bans: &[(BanTarget, &str)]) -> PathBuf {
    let path = dir.join("bans.json");
    let storage = FileBanStorage::new(path.clone());
    for (target, reason) in bans {
        storage
            .add_ban(
                BanEntry::new(String::new(), target.clone(), BanSource::Console).reason(*reason),
            )
            .await
            .unwrap();
    }
    path
}

fn closed_without_answer<T: std::fmt::Debug>(result: Result<T, HarnessError>) {
    match result {
        Err(HarnessError::Closed(_) | HarnessError::Io(_)) => {}
        other => panic!("expected the proxy to close the connection silently, got {other:?}"),
    }
}

fn handshake_of(recorder: &Recorder, pick: impl Fn(&Recorded) -> bool) -> Value {
    let handshakes = recorder.filter(|e| e.kind == EventKind::ConnectionHandshake && pick(e));
    assert_eq!(handshakes.len(), 1, "{:#?}", recorder.events());
    handshakes[0].detail.clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_handshake_event_describes_the_connection_before_the_login() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .patch_config(receive_proxy_protocol)
        .start()
        .await
        .unwrap();

    let session = FakeClient::new(proxy.addr(), VERSION)
        .domain("Lobby.Test\0FML3\0")
        .port(25577)
        .proxy_protocol(from("203.0.113.9", 50001))
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();

    let handshake = handshake_of(&recorder, |_| true);
    assert_eq!(
        handshake,
        json!({
            "remote_addr": "203.0.113.9:50001",
            "virtual_host": "lobby.test",
            "raw_host": "Lobby.Test\u{0}FML3\u{0}",
            "port": 25577,
            "protocol_version": CURRENT,
            "intent": "login",
            "legacy": false,
            "server": "lobby",
            "result": "allow",
        })
    );
    let events = recorder.events();
    let at = |kind: EventKind| events.iter().position(|e| e.kind == kind).unwrap();
    assert!(
        at(EventKind::ConnectionHandshake) < at(EventKind::PreLogin),
        "{events:#?}"
    );
    assert_eq!(recorder.count(EventKind::ConnectionRejected), 0);

    let status = FakeClient::new(proxy.addr(), VERSION)
        .domain("lobby.test")
        .proxy_protocol(from("203.0.113.9", 50002))
        .status()
        .await
        .unwrap();
    assert!(status.json.get("description").is_some(), "{status:?}");
    let status = handshake_of(&recorder, |e| e.detail["intent"] == json!("status"));
    assert_eq!(status["remote_addr"], json!("203.0.113.9:50002"));
    assert_eq!(status["server"], json!("lobby"));

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_status_ping_to_an_unknown_domain_has_no_server() {
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").unreachable())
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    FakeClient::new(proxy.addr(), VERSION)
        .domain("nowhere.test")
        .status()
        .await
        .unwrap();

    let handshake = handshake_of(&recorder, |_| true);
    assert_eq!(handshake["virtual_host"], json!("nowhere.test"));
    assert_eq!(handshake["server"], Value::Null);
    assert_eq!(handshake["intent"], json!("status"));
    settle(&proxy).await;
    assert_eq!(recorder.count(EventKind::ConnectionRejected), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_denied_handshake_disconnects_the_login_with_the_reason() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .server(ServerSpec::offline("guarded").backend(backend.addr()))
        .plugin(gatekeeper(guarded))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let info = proxy
        .client_for("guarded", VERSION)
        .unwrap()
        .login("Steve")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(info.state, ConnectionState::Login, "{info:?}");
    assert_eq!(info.text, "No bots allowed");
    assert_eq!(
        handshake_of(&recorder, |_| true)["result"],
        json!({ "deny": "No bots allowed" })
    );
    settle(&proxy).await;
    let rejected = recorder.of(EventKind::ConnectionRejected);
    assert_eq!(rejected.len(), 1, "{:#?}", recorder.events());
    assert_eq!(rejected[0].detail["reason"], json!("plugin"));
    assert_eq!(rejected[0].detail["plugin"], json!(GATEKEEPER));
    assert_eq!(rejected[0].detail["virtual_host"], json!("guarded.test"));
    assert_eq!(recorder.count(EventKind::PreLogin), 0);
    assert_eq!(backend.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_handshake_closes_the_connection_without_an_answer() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("guarded")
                .backend(backend.addr())
                .domain("silent.test"),
        )
        .plugin(gatekeeper(guarded))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    closed_without_answer(proxy.client(VERSION).login("Steve").await);
    closed_without_answer(proxy.client(VERSION).status().await);

    let handshakes = recorder.of(EventKind::ConnectionHandshake);
    assert_eq!(handshakes.len(), 2, "{:#?}", recorder.events());
    for handshake in &handshakes {
        assert_eq!(handshake.detail["result"], json!("drop_silently"));
    }
    settle(&proxy).await;
    assert_eq!(recorder.count(EventKind::ConnectionRejected), 2);
    assert_eq!(recorder.count(EventKind::PreLogin), 0);
    assert_eq!(recorder.count(EventKind::ProxyPing), 0);
    assert_eq!(backend.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_denied_status_ping_is_closed_without_an_answer() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("guarded").backend(backend.addr()))
        .plugin(gatekeeper(guarded))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    closed_without_answer(proxy.client(VERSION).status().await);

    assert_eq!(
        handshake_of(&recorder, |_| true)["result"],
        json!({ "deny": "No bots allowed" })
    );
    settle(&proxy).await;
    assert_eq!(recorder.count(EventKind::ProxyPing), 0);
    assert_eq!(recorder.count(EventKind::ConnectionRejected), 1);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_legacy_ping_goes_through_the_handshake_event() {
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("old").unreachable())
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    proxy
        .legacy_client_for("old")
        .unwrap()
        .port(25570)
        .ping()
        .await
        .unwrap();

    let handshake = handshake_of(&recorder, |_| true);
    assert_eq!(handshake["legacy"], json!(true));
    assert_eq!(handshake["intent"], json!("status"));
    assert_eq!(handshake["server"], json!("old"));
    assert_eq!(handshake["virtual_host"], json!("old.test"));
    assert_eq!(handshake["port"], json!(25570));
    assert_eq!(
        handshake["protocol_version"],
        json!(i32::from(LEGACY_PROTOCOL))
    );

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_refused_connection_is_reported_once_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let bans = ban_file(
        dir.path(),
        &[
            (BanTarget::Ip("203.0.113.7".parse().unwrap()), "address"),
            (BanTarget::Username("Mallory".into()), "name"),
        ],
    )
    .await;
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .server(ServerSpec::offline("guarded").backend(backend.addr()))
        .plugin(gatekeeper(guarded))
        .plugin(recorder.plugin())
        .patch_config(move |table| {
            receive_proxy_protocol(table);
            table.insert("unknown_domain_behavior".into(), "drop".into());
            let rate_limit: Table = toml::from_str(
                "enabled = true\nmax_connections = 1\nwindow = \"1h\"\nstatus_max = 100\nstatus_window = \"1h\"",
            )
            .unwrap();
            table.insert("rate_limit".into(), rate_limit.into());
            let ip_filter: Table = toml::from_str("blacklist = [\"203.0.113.66/32\"]").unwrap();
            table.insert("ip_filter".into(), ip_filter.into());
            table
                .get_mut("ban")
                .and_then(toml::Value::as_table_mut)
                .unwrap()
                .insert("file".into(), bans.to_str().unwrap().into());
        })
        .start()
        .await
        .unwrap();
    let client = |ip: &str, port: u16| proxy.client(VERSION).proxy_protocol(from(ip, port));

    let alice = client("198.51.100.1", 50001)
        .login("Alice")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _alice_conn = backend.next_connection(T).await.unwrap();

    closed_without_answer(client("203.0.113.66", 50002).login("Eve").await);
    closed_without_answer(client("198.51.100.1", 50003).login("Alice").await);
    closed_without_answer(
        FakeClient::new(proxy.addr(), VERSION)
            .domain("nowhere.test")
            .proxy_protocol(from("198.51.100.4", 50004))
            .login("Bob")
            .await,
    );
    let address = client("203.0.113.7", 50005)
        .login("Trudy")
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert!(address.text.contains("address"), "{address:?}");
    let name = client("198.51.100.6", 50006)
        .login("Mallory")
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert!(name.text.contains("name"), "{name:?}");
    let denied = proxy
        .client_for("guarded", VERSION)
        .unwrap()
        .proxy_protocol(from("198.51.100.8", 50008))
        .login("Carol")
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(denied.text, "No bots allowed");

    settle(&proxy).await;
    let rejected: Vec<Value> = recorder
        .of(EventKind::ConnectionRejected)
        .into_iter()
        .map(|e| e.detail)
        .collect();
    assert_eq!(
        rejected,
        [
            json!({ "remote_addr": "203.0.113.66:50002", "virtual_host": null, "reason": "ip_filter", "plugin": null }),
            json!({ "remote_addr": "198.51.100.1:50003", "virtual_host": "lobby.test", "reason": "rate_limit", "plugin": null }),
            json!({ "remote_addr": "198.51.100.4:50004", "virtual_host": "nowhere.test", "reason": "unknown_domain", "plugin": null }),
            json!({ "remote_addr": "203.0.113.7:50005", "virtual_host": "lobby.test", "reason": "ip_banned", "plugin": null }),
            json!({ "remote_addr": "198.51.100.6:50006", "virtual_host": "lobby.test", "reason": "banned", "plugin": null }),
            json!({ "remote_addr": "198.51.100.8:50008", "virtual_host": "guarded.test", "reason": "plugin", "plugin": GATEKEEPER }),
        ]
    );
    assert_eq!(recorder.count(EventKind::PostLogin), 1);

    alice.quit().await;
    proxy.shutdown().await.unwrap();
}
