#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use infrarust_api::event::EventPriority;
use infrarust_api::events::connection::{KickedFromServerEvent, PlayerChooseInitialServerEvent};
use infrarust_api::events::lifecycle::PreLoginEvent;
use infrarust_api::player::ConnectionResult;
use infrarust_api::types::{Component, ServerId};
use infrarust_config::ProxyMode;
use infrarust_server_manager::ServerState;
use infrarust_test_harness::recorder::component_value;
use infrarust_test_harness::versions::CURRENT;
use infrarust_test_harness::{
    DEFAULT_TIMEOUT, EventKind, FakeBackend, FakeServerProvider, ProtocolVersion, Recorder,
    ScriptedPlugin, ServerSpec, TestProxy, version_matrix,
};
use serde_json::json;

const T: Duration = DEFAULT_TIMEOUT;
const STEVE: &str = "Steve";
const UNAVAILABLE: &str = "Server is unavailable. Please try again later.";

macro_rules! modes {
    ($body:ident) => {
        mod $body {
            use super::*;

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn offline() {
                super::$body(ProxyMode::Offline).await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn passthrough() {
                super::$body(ProxyMode::Passthrough).await;
            }
        }
    };
}

fn server(id: &str, mode: ProxyMode, backend: &FakeBackend) -> ServerSpec {
    let spec = ServerSpec::new(id, mode).backend(backend.addr());
    if mode == ProxyMode::Offline {
        spec.network("main")
    } else {
        spec
    }
}

fn boot_when_started(provider: &Arc<FakeServerProvider>) {
    let provider = Arc::clone(provider);
    tokio::spawn(async move {
        if provider.wait_for_start(T).await.is_ok() {
            provider.boot();
        }
    });
}

fn state(proxy: &TestProxy, server: &str) -> Option<ServerState> {
    proxy
        .services()
        .server_manager
        .as_ref()
        .and_then(|manager| manager.get_state(server))
}

async fn a_denied_login_leaves_a_sleeping_server_asleep(mode: ProxyMode) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let provider = FakeServerProvider::sleeping();
    boot_when_started(&provider);
    let gate = ScriptedPlugin::new("gate").on::<PreLoginEvent>(EventPriority::NORMAL, |e| {
        e.deny(Component::text("Whitelist only"));
    });
    let proxy = TestProxy::builder()
        .server(server("lobby", mode, &backend).managed(Arc::clone(&provider)))
        .plugin(gate)
        .start()
        .await
        .unwrap();

    let info = proxy
        .client_for("lobby", ProtocolVersion(CURRENT))
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(info.text, "Whitelist only");
    assert_eq!(
        provider.starts(),
        0,
        "a refused player must not wake the server"
    );
    assert_eq!(state(&proxy, "lobby"), Some(ServerState::Sleeping));
    assert_eq!(backend.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

modes!(a_denied_login_leaves_a_sleeping_server_asleep);

async fn a_redirect_away_from_a_sleeping_server_leaves_it_asleep(mode: ProxyMode) {
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let hub = FakeBackend::builder().spawn().await.unwrap();
    let provider = FakeServerProvider::sleeping();
    boot_when_started(&provider);
    let hub_first = ScriptedPlugin::new("hub_first")
        .on::<PlayerChooseInitialServerEvent>(EventPriority::NORMAL, |e| {
            e.redirect_to(ServerId::new("hub"))
        });
    let proxy = TestProxy::builder()
        .server(server("lobby", mode, &lobby).managed(Arc::clone(&provider)))
        .server(server("hub", mode, &hub))
        .plugin(hub_first)
        .start()
        .await
        .unwrap();

    let session = proxy
        .client_for("lobby", ProtocolVersion(CURRENT))
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _hub_conn = hub.next_connection(T).await.unwrap();

    assert_eq!(
        provider.starts(),
        0,
        "the server the player was redirected away from must stay asleep"
    );
    assert_eq!(state(&proxy, "lobby"), Some(ServerState::Sleeping));
    assert_eq!(lobby.accepted_connections(), 0);

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

modes!(a_redirect_away_from_a_sleeping_server_leaves_it_asleep);

async fn a_login_wakes_the_server_once_plugins_chose_it(mode: ProxyMode) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let provider = FakeServerProvider::sleeping();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(server("lobby", mode, &backend).managed(Arc::clone(&provider)))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let client = proxy.client_for("lobby", ProtocolVersion(CURRENT)).unwrap();
    let login = tokio::spawn(async move { client.login(STEVE).await });
    provider.wait_for_start(T).await.unwrap();

    let chosen = recorder.of(EventKind::ServerPreConnect);
    assert_eq!(
        chosen.len(),
        1,
        "the server starts only after ServerPreConnectEvent picked it: {:?}",
        recorder.kinds()
    );
    assert_eq!(chosen[0].detail["server"], json!("lobby"));
    assert_eq!(recorder.count(EventKind::PostLogin), 1);
    assert_eq!(backend.accepted_connections(), 0);

    provider.boot();
    let session = tokio::time::timeout(T, login)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();
    recorder
        .wait_for(
            |e| e.kind == EventKind::ServerConnected && e.detail["server"] == json!("lobby"),
            T,
        )
        .await
        .unwrap();
    assert_eq!(provider.starts(), 1);
    assert_eq!(state(&proxy, "lobby"), Some(ServerState::Online));
    assert_eq!(recorder.count(EventKind::KickedFromServer), 0);

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

modes!(a_login_wakes_the_server_once_plugins_chose_it);

async fn a_server_that_cannot_start_is_a_kick_plugins_can_redirect(mode: ProxyMode) {
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let hub = FakeBackend::builder().spawn().await.unwrap();
    let provider = FakeServerProvider::broken();
    let recorder = Recorder::new();
    let rescue = ScriptedPlugin::new("rescue")
        .on::<KickedFromServerEvent>(EventPriority::NORMAL, |e| {
            e.redirect_to(ServerId::new("hub"))
        });
    let proxy = TestProxy::builder()
        .server(server("lobby", mode, &lobby).managed(Arc::clone(&provider)))
        .server(server("hub", mode, &hub))
        .plugin(rescue)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let session = proxy
        .client_for("lobby", ProtocolVersion(CURRENT))
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _hub_conn = hub.next_connection(T).await.unwrap();
    recorder
        .wait_for(
            |e| e.kind == EventKind::ServerConnected && e.detail["server"] == json!("hub"),
            T,
        )
        .await
        .unwrap();

    let kicks = recorder.of(EventKind::KickedFromServer);
    assert_eq!(kicks.len(), 1, "{kicks:?}");
    assert_eq!(kicks[0].detail["server"], json!("lobby"));
    assert_eq!(kicks[0].detail["cause"], json!("unreachable"));
    assert_eq!(kicks[0].detail["during_connect"], json!(true));
    assert_eq!(
        kicks[0].detail["reason"],
        component_value(&Component::text(UNAVAILABLE))
    );
    assert_eq!(recorder.count(EventKind::ConnectionRejected), 0);
    assert_eq!(provider.starts(), 1);
    assert_eq!(lobby.accepted_connections(), 0);

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

modes!(a_server_that_cannot_start_is_a_kick_plugins_can_redirect);

async fn a_server_that_cannot_start_disconnects_with_its_message(mode: ProxyMode) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let provider = FakeServerProvider::broken();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(server("lobby", mode, &backend).managed(Arc::clone(&provider)))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let info = proxy
        .client_for("lobby", ProtocolVersion(CURRENT))
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(info.text, UNAVAILABLE);
    recorder
        .wait_for(|e| e.kind == EventKind::Disconnect, T)
        .await
        .unwrap();
    assert_eq!(recorder.count(EventKind::KickedFromServer), 1);
    assert_eq!(backend.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

modes!(a_server_that_cannot_start_disconnects_with_its_message);

async fn a_switch_to_a_sleeping_server_wakes_it(version: ProtocolVersion) {
    let hub = FakeBackend::builder().spawn().await.unwrap();
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let provider = FakeServerProvider::sleeping();
    let proxy = TestProxy::builder()
        .server(server("hub", ProxyMode::Offline, &hub))
        .server(server("lobby", ProxyMode::Offline, &lobby).managed(Arc::clone(&provider)))
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client_for("hub", version)
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _hub_conn = hub.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();

    let switching = Arc::clone(&player);
    let switch = tokio::spawn(async move { switching.connect(ServerId::new("lobby")).await });
    provider.wait_for_start(T).await.unwrap();
    assert_eq!(lobby.accepted_connections(), 0);
    provider.boot();

    let result = tokio::time::timeout(T, switch)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result, ConnectionResult::Success);
    session.expect_join(T).await.unwrap();
    let _lobby_conn = lobby.next_connection(T).await.unwrap();
    assert_eq!(player.current_server(), Some(ServerId::new("lobby")));
    assert_eq!(state(&proxy, "lobby"), Some(ServerState::Online));

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(a_switch_to_a_sleeping_server_wakes_it; p340 = 340, p774 = 774);
