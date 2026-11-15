#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use infrarust_api::event::{EventPriority, ResultedEvent};
use infrarust_api::events::connection::{
    PlayerChooseInitialServerEvent, PlayerChooseInitialServerResult, ServerPreConnectEvent,
    ServerPreConnectResult,
};
use infrarust_api::events::lifecycle::{PreLoginEvent, PreLoginResult};
use infrarust_api::limbo::handler::LimboOutcome;
use infrarust_api::player::{ConnectionResult, Player};
use infrarust_api::types::{Component, ServerId};
use infrarust_test_harness::{
    BackendConn, ClientSession, DEFAULT_TIMEOUT, FakeBackend, FakeSessionServer, ProtocolVersion,
    ScriptedPlugin, ServerSpec, TestProxy, holding_gate, next_hold,
};

const T: Duration = DEFAULT_TIMEOUT;
const VERSION: ProtocolVersion = ProtocolVersion(774);
const GATE: &str = "gate";
const REFUSED: &str = "This server requires a verified Minecraft account";
const STEVE: &str = "Steve";

fn server(name: &str) -> ServerId {
    ServerId::new(name)
}

fn redirect_pre_connect(from: &'static str, to: &'static str) -> ScriptedPlugin {
    ScriptedPlugin::new("redirector").on::<ServerPreConnectEvent>(
        EventPriority::NORMAL,
        move |event| {
            if event.server.as_str() == from {
                event.set_result(ServerPreConnectResult::Redirect(server(to)));
            }
        },
    )
}

fn force_offline() -> ScriptedPlugin {
    ScriptedPlugin::new("force-offline").on::<PreLoginEvent>(EventPriority::NORMAL, |event| {
        event.set_result(PreLoginResult::ForceOffline);
    })
}

async fn join(proxy: &TestProxy, entry: &str, backend: &FakeBackend) -> Joined {
    let session = proxy
        .client_for(entry, VERSION)
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let conn = backend.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    Joined {
        session,
        _conn: conn,
        player,
    }
}

struct Joined {
    session: ClientSession,
    _conn: BackendConn,
    player: Arc<dyn Player>,
}

fn assert_refused(result: &ConnectionResult) {
    assert_eq!(
        *result,
        ConnectionResult::Denied(Component::text(REFUSED)),
        "{result:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_offline_player_cannot_switch_to_a_client_only_server() {
    let open = FakeBackend::builder().spawn().await.unwrap();
    let secure = FakeBackend::builder().spawn().await.unwrap();
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("open").backend(open.addr()))
        .server(ServerSpec::client_only("secure").backend(secure.addr()))
        .session_server(&sessions)
        .start()
        .await
        .unwrap();
    let mut joined = join(&proxy, "open", &open).await;

    let result = joined.player.connect(server("secure")).await.unwrap();

    assert_refused(&result);
    assert_eq!(secure.accepted_connections(), 0);
    assert_eq!(joined.player.current_server(), Some(server("open")));
    assert_eq!(joined.session.expect_system_text(T).await.unwrap(), REFUSED);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plugin_redirect_cannot_carry_an_offline_player_to_a_client_only_server() {
    let open = FakeBackend::builder().spawn().await.unwrap();
    let other = FakeBackend::builder().spawn().await.unwrap();
    let secure = FakeBackend::builder().spawn().await.unwrap();
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("open").backend(open.addr()))
        .server(ServerSpec::offline("other").backend(other.addr()))
        .server(ServerSpec::client_only("secure").backend(secure.addr()))
        .session_server(&sessions)
        .plugin(redirect_pre_connect("other", "secure"))
        .start()
        .await
        .unwrap();
    let joined = join(&proxy, "open", &open).await;

    let result = joined.player.connect(server("other")).await.unwrap();

    assert_refused(&result);
    assert_eq!(secure.accepted_connections(), 0);
    assert_eq!(other.accepted_connections(), 0);
    assert_eq!(joined.player.current_server(), Some(server("open")));

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_limbo_exit_cannot_carry_an_offline_player_to_a_client_only_server() {
    let open = FakeBackend::builder().spawn().await.unwrap();
    let secure = FakeBackend::builder().spawn().await.unwrap();
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let (gate, mut holds) = holding_gate("gatekeeper", GATE);
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("open")
                .backend(open.addr())
                .limbo_handlers([GATE]),
        )
        .server(ServerSpec::client_only("secure").backend(secure.addr()))
        .session_server(&sessions)
        .plugin(gate)
        .start()
        .await
        .unwrap();
    let mut session = proxy
        .client_for("open", VERSION)
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let handle = next_hold(&mut holds, T).await.unwrap().handle;

    handle.complete(LimboOutcome::Redirect(server("secure")));

    let info = session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.text, REFUSED, "{info:?}");
    assert_eq!(secure.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

async fn an_initial_redirect_is_refused(plugin: ScriptedPlugin) {
    let open = FakeBackend::builder().spawn().await.unwrap();
    let secure = FakeBackend::builder().spawn().await.unwrap();
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("open").backend(open.addr()))
        .server(ServerSpec::client_only("secure").backend(secure.addr()))
        .session_server(&sessions)
        .plugin(plugin)
        .start()
        .await
        .unwrap();

    let info = proxy
        .client_for("open", VERSION)
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(info.text, REFUSED, "{info:?}");
    assert_eq!(secure.accepted_connections(), 0);
    assert_eq!(open.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_initial_server_choice_cannot_carry_an_offline_player_to_a_client_only_server() {
    an_initial_redirect_is_refused(
        ScriptedPlugin::new("chooser").on::<PlayerChooseInitialServerEvent>(
            EventPriority::NORMAL,
            |event| {
                event.set_result(PlayerChooseInitialServerResult::Redirect(server("secure")));
            },
        ),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_initial_pre_connect_cannot_carry_an_offline_player_to_a_client_only_server() {
    an_initial_redirect_is_refused(redirect_pre_connect("open", "secure")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_force_offline_player_keeps_the_entry_server_and_only_it() {
    let entry = FakeBackend::builder().spawn().await.unwrap();
    let vault = FakeBackend::builder().spawn().await.unwrap();
    let open = FakeBackend::builder().spawn().await.unwrap();
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("entry").backend(entry.addr()))
        .server(ServerSpec::client_only("vault").backend(vault.addr()))
        .server(ServerSpec::offline("open").backend(open.addr()))
        .session_server(&sessions)
        .plugin(force_offline())
        .start()
        .await
        .unwrap();
    let joined = join(&proxy, "entry", &entry).await;
    assert!(!joined.player.is_online_mode());

    assert_refused(&joined.player.connect(server("vault")).await.unwrap());
    assert_eq!(vault.accepted_connections(), 0);

    assert_eq!(
        joined.player.connect(server("open")).await.unwrap(),
        ConnectionResult::Success
    );
    let _open_conn = open.next_connection(T).await.unwrap();

    assert_refused(&joined.player.connect(server("entry")).await.unwrap());
    assert_eq!(entry.accepted_connections(), 1);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_force_offline_player_passes_the_limbo_gate_of_the_entry_server() {
    let entry = FakeBackend::builder().spawn().await.unwrap();
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let (gate, mut holds) = holding_gate("gatekeeper", GATE);
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::client_only("entry")
                .backend(entry.addr())
                .limbo_handlers([GATE]),
        )
        .session_server(&sessions)
        .plugin(gate)
        .plugin(force_offline())
        .start()
        .await
        .unwrap();
    let mut session = proxy
        .client_for("entry", VERSION)
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let handle = next_hold(&mut holds, T).await.unwrap().handle;

    handle.complete(LimboOutcome::Accept);

    let _conn = entry.next_connection(T).await.unwrap();
    session.expect_join(T).await.unwrap();

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_verified_player_moves_between_client_only_servers() {
    let one = FakeBackend::builder().spawn().await.unwrap();
    let two = FakeBackend::builder().spawn().await.unwrap();
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("one").backend(one.addr()))
        .server(ServerSpec::client_only("two").backend(two.addr()))
        .session_server(&sessions)
        .start()
        .await
        .unwrap();
    let joined = join(&proxy, "one", &one).await;
    assert!(joined.player.is_online_mode());

    assert_eq!(
        joined.player.connect(server("two")).await.unwrap(),
        ConnectionResult::Success
    );
    assert_eq!(two.accepted_connections(), 1);

    proxy.shutdown().await.unwrap();
}
