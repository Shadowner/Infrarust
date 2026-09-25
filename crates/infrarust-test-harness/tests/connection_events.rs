#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::event::{BoxFuture, EventPriority};
use infrarust_api::events::connection::{
    PlayerChooseInitialServerEvent, ServerConnectedEvent, ServerPostConnectEvent,
    ServerPreConnectEvent,
};
use infrarust_api::events::proxy::ServerStateChangeEvent;
use infrarust_api::limbo::handle::SessionHandle;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::player::Player;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::services::server_manager::ServerState;
use infrarust_api::types::{Component, ServerId};
use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_plugin_admin_api::sse::event_bridge::EventBridge;
use infrarust_plugin_admin_api::state::ApiEvent;
use infrarust_plugin_server_wake::ServerWakePlugin;
use infrarust_protocol::packets::play::chat::SChatMessage;
use infrarust_test_harness::versions::CURRENT;
use infrarust_test_harness::{
    ClientSession, DEFAULT_TIMEOUT, EventKind, FakeBackend, FakeSessionServer, LoginBehavior,
    ProtocolVersion, Recorded, Recorder, ScriptedPlugin, ServerSpec, TestProxy, version_matrix,
};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use toml::Table;
use uuid::Uuid;

const T: Duration = DEFAULT_TIMEOUT;
const GATE: &str = "gate";
const STEVE: &str = "Steve";

type Sightings = Arc<Mutex<Vec<Sighting>>>;
type Counters = Vec<(&'static str, Arc<AtomicUsize>)>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Sighting {
    event: &'static str,
    server: String,
    previous: Option<String>,
    current: Option<String>,
    cause: Option<&'static str>,
    accepted: Option<usize>,
    username: String,
    uuid: Uuid,
}

impl Sighting {
    fn new(
        event: &'static str,
        player: &dyn Player,
        server: &ServerId,
        previous: Option<&ServerId>,
    ) -> Self {
        Self {
            event,
            server: server.as_str().to_string(),
            previous: previous.map(|s| s.as_str().to_string()),
            current: player.current_server().map(|s| s.as_str().to_string()),
            cause: None,
            accepted: None,
            username: player.profile().username.clone(),
            uuid: player.profile().uuid,
        }
    }
}

fn expect(
    event: &'static str,
    server: &str,
    previous: Option<&str>,
    current: Option<&str>,
) -> Sighting {
    Sighting {
        event,
        server: server.to_string(),
        previous: previous.map(str::to_string),
        current: current.map(str::to_string),
        cause: None,
        accepted: None,
        username: STEVE.to_string(),
        uuid: offline_uuid(STEVE),
    }
}

fn expect_pre_connect(
    server: &str,
    previous: Option<&str>,
    current: Option<&str>,
    cause: &'static str,
) -> Sighting {
    Sighting {
        cause: Some(cause),
        accepted: Some(0),
        ..expect("pre_connect", server, previous, current)
    }
}

fn probe(sightings: &Sightings, counters: Counters) -> ScriptedPlugin {
    let counters = Arc::new(counters);
    let chosen = Arc::clone(sightings);
    let pre = Arc::clone(sightings);
    let connected = Arc::clone(sightings);
    let joined = Arc::clone(sightings);
    ScriptedPlugin::new("probe")
        .on::<PlayerChooseInitialServerEvent>(EventPriority::NORMAL, move |e| {
            let seen = Sighting::new("choose", e.player.as_ref(), &e.initial_server, None);
            chosen.lock().unwrap().push(seen);
        })
        .on::<ServerPreConnectEvent>(EventPriority::NORMAL, move |e| {
            let accepted = counters
                .iter()
                .find(|(id, _)| *id == e.server.as_str())
                .map(|(_, counter)| counter.load(Ordering::SeqCst));
            let seen = Sighting {
                cause: Some(e.cause.as_str()),
                accepted,
                ..Sighting::new(
                    "pre_connect",
                    e.player.as_ref(),
                    &e.server,
                    e.previous_server.as_ref(),
                )
            };
            pre.lock().unwrap().push(seen);
        })
        .on::<ServerConnectedEvent>(EventPriority::NORMAL, move |e| {
            let seen = Sighting::new(
                "connected",
                e.player.as_ref(),
                &e.server,
                e.previous_server.as_ref(),
            );
            connected.lock().unwrap().push(seen);
        })
        .on::<ServerPostConnectEvent>(EventPriority::NORMAL, move |e| {
            let seen = Sighting::new(
                "post_connect",
                e.player.as_ref(),
                &e.server,
                e.previous_server.as_ref(),
            );
            joined.lock().unwrap().push(seen);
        })
}

fn seen(sightings: &Sightings) -> Vec<Sighting> {
    sightings.lock().unwrap().clone()
}

fn named(event: &Recorded, username: &str) -> bool {
    event.username.as_deref() == Some(username)
}

fn connection_kinds(recorder: &Recorder) -> Vec<EventKind> {
    recorder
        .for_username(STEVE)
        .iter()
        .map(|e| e.kind)
        .filter(|kind| {
            matches!(
                kind,
                EventKind::PostLogin
                    | EventKind::PlayerChooseInitialServer
                    | EventKind::ServerPreConnect
                    | EventKind::ServerConnected
                    | EventKind::ServerPostConnect
                    | EventKind::Disconnect
            )
        })
        .collect()
}

async fn sync(session: &mut ClientSession, player: &dyn Player) {
    player.send_message(Component::text("sync")).unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "sync");
}

fn network(spec: ServerSpec) -> ServerSpec {
    spec.network("main")
}

async fn assert_initial_events(
    proxy: TestProxy,
    recorder: &Recorder,
    sightings: &Sightings,
    backend: &FakeBackend,
    version: ProtocolVersion,
) {
    let session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let conn = backend.next_connection(T).await.unwrap();
    session.quit().await;
    let disconnect = recorder
        .wait_for(|e| e.kind == EventKind::Disconnect && named(e, STEVE), T)
        .await
        .unwrap();
    conn.closed(T).await.unwrap();

    assert_eq!(
        connection_kinds(recorder),
        [
            EventKind::PostLogin,
            EventKind::PlayerChooseInitialServer,
            EventKind::ServerPreConnect,
            EventKind::ServerConnected,
            EventKind::ServerPostConnect,
            EventKind::Disconnect,
        ],
        "protocol {}",
        version.0
    );
    assert_eq!(
        seen(sightings),
        [
            expect("choose", "lobby", None, None),
            expect_pre_connect("lobby", None, None, "initial"),
            expect("connected", "lobby", None, None),
            expect("post_connect", "lobby", None, Some("lobby")),
        ]
    );
    let player = disconnect.player.expect("Disconnect carries the player");
    for event in recorder.filter(|e| named(e, STEVE) && e.player.is_some()) {
        assert_eq!(event.player, Some(player), "{}", event.kind);
    }
    assert_eq!(disconnect.detail["last_server"], json!("lobby"));
    assert_eq!(backend.accepted_connections(), 1);

    proxy.shutdown().await.unwrap();
}

async fn initial_connection_events_carry_the_player(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let sightings = Sightings::default();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(probe(&sightings, vec![("lobby", backend.accept_counter())]))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    assert_initial_events(proxy, &recorder, &sightings, &backend, version).await;
}

version_matrix!(LIFECYCLE, initial_connection_events_carry_the_player);

async fn client_only_connection_events_carry_the_player(version: ProtocolVersion) {
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let sightings = Sightings::default();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("lobby").backend(backend.addr()))
        .session_server(&sessions)
        .plugin(probe(&sightings, vec![("lobby", backend.accept_counter())]))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    assert_initial_events(proxy, &recorder, &sightings, &backend, version).await;
}

version_matrix!(client_only_connection_events_carry_the_player;
    p47 = 47, p340 = 340, p763 = 763, p764 = 764, p766 = 766, p774 = 774);

async fn a_switch_carries_the_previous_server(version: ProtocolVersion) {
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let sightings = Sightings::default();
    let counters = vec![
        ("a", backend_a.accept_counter()),
        ("b", backend_b.accept_counter()),
    ];
    let proxy = TestProxy::builder()
        .server(network(ServerSpec::offline("a").backend(backend_a.addr())))
        .server(network(ServerSpec::offline("b").backend(backend_b.addr())))
        .plugin(probe(&sightings, counters))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let conn_a = backend_a.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    let joined_a = recorder
        .wait_for(|e| e.kind == EventKind::ServerPostConnect, T)
        .await
        .unwrap();

    player.switch_server(ServerId::new("b")).await.unwrap();
    session.expect_join(T).await.unwrap();
    let mut conn_b = backend_b.next_connection(T).await.unwrap();
    conn_a.closed(T).await.unwrap();
    sync(&mut session, player.as_ref()).await;

    assert_eq!(
        seen(&sightings)[4..],
        [
            expect_pre_connect("b", Some("a"), Some("a"), "switch"),
            expect("connected", "b", Some("a"), Some("a")),
            expect("post_connect", "b", Some("a"), Some("b")),
        ]
    );
    let after: Vec<EventKind> = recorder
        .filter(|e| e.seq > joined_a.seq)
        .iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        after,
        [
            EventKind::ServerPreConnect,
            EventKind::ServerConnected,
            EventKind::ServerPostConnect,
        ]
    );
    assert_eq!(player.current_server(), Some(ServerId::new("b")));
    session.chat("hello b").await.unwrap();
    assert_eq!(
        conn_b.expect::<SChatMessage>(T).await.unwrap().message,
        "hello b"
    );

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(SWITCH, a_switch_carries_the_previous_server);

async fn a_switch_to_the_current_server_fires_nothing(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(network(ServerSpec::offline("a").backend(backend.addr())))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();

    player.switch_server(ServerId::new("a")).await.unwrap();
    sync(&mut session, player.as_ref()).await;

    for kind in [
        EventKind::ServerPreConnect,
        EventKind::ServerConnected,
        EventKind::ServerPostConnect,
    ] {
        assert_eq!(recorder.count(kind), 1, "{kind}: {:?}", recorder.events());
    }
    assert_eq!(backend.accepted_connections(), 1);
    session.chat("still on a").await.unwrap();
    assert_eq!(
        conn.expect::<SChatMessage>(T).await.unwrap().message,
        "still on a"
    );

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(SWITCH, a_switch_to_the_current_server_fires_nothing);

async fn a_refused_backend_login_is_never_connected(version: ProtocolVersion) {
    let backend = FakeBackend::builder()
        .login(LoginBehavior::refuse_text("go away"))
        .spawn()
        .await
        .unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let info = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(info.text, "go away", "{info:?}");
    let disconnect = recorder
        .wait_for(|e| e.kind == EventKind::Disconnect && named(e, STEVE), T)
        .await
        .unwrap();

    assert_eq!(recorder.count(EventKind::ServerPreConnect), 1);
    assert_eq!(recorder.count(EventKind::ServerConnected), 0);
    assert_eq!(recorder.count(EventKind::ServerPostConnect), 0);
    assert_eq!(disconnect.detail["last_server"], json!(null));
    proxy.shutdown().await.unwrap();
}

version_matrix!(SWITCH, a_refused_backend_login_is_never_connected);

async fn a_hung_backend_login_is_never_connected(version: ProtocolVersion) {
    let backend = FakeBackend::builder()
        .login(LoginBehavior::Hang)
        .spawn()
        .await
        .unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let client = proxy.client(version);
    let login = tokio::spawn(async move { client.login(STEVE).await });
    let conn = backend.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    player.disconnect(Component::text("bye")).await;
    tokio::time::timeout(T, login)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .disconnected()
        .unwrap();
    conn.closed(T).await.unwrap();
    recorder
        .wait_for(|e| e.kind == EventKind::Disconnect && named(e, STEVE), T)
        .await
        .unwrap();

    assert_eq!(recorder.count(EventKind::ServerConnected), 0);
    assert_eq!(recorder.count(EventKind::ServerPostConnect), 0);
    proxy.shutdown().await.unwrap();
}

version_matrix!(SWITCH, a_hung_backend_login_is_never_connected);

async fn a_refused_switch_keeps_the_player_where_they_are(version: ProtocolVersion) {
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder()
        .login(LoginBehavior::refuse_text("full"))
        .spawn()
        .await
        .unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(network(ServerSpec::offline("a").backend(backend_a.addr())))
        .server(network(ServerSpec::offline("b").backend(backend_b.addr())))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn_a = backend_a.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();

    player.switch_server(ServerId::new("b")).await.unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "full");
    let _refused = backend_b.next_connection(T).await.unwrap();

    let to_b = |e: &Recorded| e.detail["server"] == json!("b");
    let pre_connects = recorder.filter(|e| e.kind == EventKind::ServerPreConnect && to_b(e));
    assert_eq!(pre_connects.len(), 1, "{pre_connects:?}");
    assert_eq!(pre_connects[0].detail["cause"], json!("switch"));
    let joined_b = recorder.filter(|e| {
        matches!(
            e.kind,
            EventKind::ServerConnected | EventKind::ServerPostConnect
        ) && to_b(e)
    });
    assert!(joined_b.is_empty(), "{joined_b:?}");
    assert_eq!(player.current_server(), Some(ServerId::new("a")));
    session.chat("still on a").await.unwrap();
    assert_eq!(
        conn_a.expect::<SChatMessage>(T).await.unwrap().message,
        "still on a"
    );

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(SWITCH, a_refused_switch_keeps_the_player_where_they_are);

struct Gate {
    held: mpsc::UnboundedSender<SessionHandle>,
}

impl LimboHandler for Gate {
    fn name(&self) -> &str {
        GATE
    }

    fn on_player_enter<'a>(
        &'a self,
        session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        let _ = self.held.send(session.handle());
        Box::pin(async { HandlerResult::Hold })
    }
}

fn gatekeeper() -> (ScriptedPlugin, mpsc::UnboundedReceiver<SessionHandle>) {
    let (held, holds) = mpsc::unbounded_channel();
    let plugin = ScriptedPlugin::new("gatekeeper").on_enable(move |ctx| {
        ctx.register_limbo_handler(Box::new(Gate { held: held.clone() }));
    });
    (plugin, holds)
}

async fn a_limbo_gate_on_the_initial_server_connects_once(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let sightings = Sightings::default();
    let (gate, mut holds) = gatekeeper();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("lobby")
                .backend(backend.addr())
                .limbo_handlers([GATE]),
        )
        .plugin(gate)
        .plugin(probe(&sightings, vec![("lobby", backend.accept_counter())]))
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let handle = tokio::time::timeout(T, holds.recv())
        .await
        .unwrap()
        .unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    let lobby = ServerId::new("lobby");
    let registry = &proxy.services().connection_registry;
    assert_eq!(player.current_server(), None);
    assert_eq!(registry.count_by_server("lobby"), 1);
    assert_eq!(proxy.services().player_registry.online_count_on(&lobby), 1);
    assert_eq!(backend.accepted_connections(), 0);

    handle.complete(HandlerResult::Accept);
    let _conn = backend.next_connection(T).await.unwrap();
    session.expect_join(T).await.unwrap();
    sync(&mut session, player.as_ref()).await;

    assert_eq!(
        seen(&sightings),
        [
            expect("choose", "lobby", None, None),
            expect_pre_connect("lobby", None, None, "initial"),
            expect("connected", "lobby", None, None),
            expect("post_connect", "lobby", None, Some("lobby")),
        ]
    );
    assert_eq!(player.current_server(), Some(lobby));
    assert_eq!(registry.count_by_server("lobby"), 1);

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(SWITCH, a_limbo_gate_on_the_initial_server_connects_once);

async fn leaving_an_initial_gate_for_another_server_is_a_limbo_exit(version: ProtocolVersion) {
    let hub = FakeBackend::builder().spawn().await.unwrap();
    let game = FakeBackend::builder().spawn().await.unwrap();
    let sightings = Sightings::default();
    let (gate, mut holds) = gatekeeper();
    let counters = vec![
        ("hub", hub.accept_counter()),
        ("game", game.accept_counter()),
    ];
    let proxy = TestProxy::builder()
        .server(network(
            ServerSpec::offline("hub")
                .backend(hub.addr())
                .limbo_handlers([GATE]),
        ))
        .server(network(ServerSpec::offline("game").backend(game.addr())))
        .plugin(gate)
        .plugin(probe(&sightings, counters))
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
    let handle = tokio::time::timeout(T, holds.recv())
        .await
        .unwrap()
        .unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();

    handle.complete(HandlerResult::Redirect(ServerId::new("game")));
    let _conn = game.next_connection(T).await.unwrap();
    session.expect_join(T).await.unwrap();
    sync(&mut session, player.as_ref()).await;

    assert_eq!(
        seen(&sightings),
        [
            expect("choose", "hub", None, None),
            expect_pre_connect("hub", None, None, "initial"),
            expect_pre_connect("game", None, None, "limbo_exit"),
            expect("connected", "game", None, None),
            expect("post_connect", "game", None, Some("game")),
        ]
    );
    assert_eq!(hub.accepted_connections(), 0);
    assert_eq!(player.current_server(), Some(ServerId::new("game")));

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(
    SWITCH,
    leaving_an_initial_gate_for_another_server_is_a_limbo_exit
);

async fn passthrough_announces_the_initial_connection(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let sightings = Sightings::default();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
        .plugin(probe(&sightings, vec![("lobby", backend.accept_counter())]))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    session.quit().await;
    conn.closed(T).await.unwrap();
    conn.close().await;
    recorder
        .wait_for(|e| e.kind == EventKind::Disconnect && named(e, STEVE), T)
        .await
        .unwrap();

    assert_eq!(
        connection_kinds(&recorder),
        [
            EventKind::PostLogin,
            EventKind::PlayerChooseInitialServer,
            EventKind::ServerPreConnect,
            EventKind::ServerConnected,
            EventKind::Disconnect,
        ]
    );
    assert_eq!(
        seen(&sightings),
        [
            expect("choose", "lobby", None, None),
            expect_pre_connect("lobby", None, None, "initial"),
            expect("connected", "lobby", None, None),
        ]
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(passthrough_announces_the_initial_connection; p47 = 47, p774 = 774);

fn sse_bridge(events: broadcast::Sender<ApiEvent>) -> ScriptedPlugin {
    ScriptedPlugin::new("sse").on_enable(move |ctx| {
        EventBridge::new(events.clone()).register_listeners(ctx);
    })
}

fn switches(events: &mut broadcast::Receiver<ApiEvent>) -> Vec<Value> {
    let mut found = Vec::new();
    while let Ok(event) = events.try_recv() {
        if event.event_type() == "player.switch" {
            let mut value = serde_json::to_value(&event).unwrap();
            value["data"]["timestamp"] = json!("-");
            found.push(value);
        }
    }
    found
}

async fn the_admin_api_reports_a_switch_but_not_a_first_join(version: ProtocolVersion) {
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().spawn().await.unwrap();
    let (events, mut received) = broadcast::channel(64);
    let proxy = TestProxy::builder()
        .server(network(ServerSpec::offline("a").backend(backend_a.addr())))
        .server(network(ServerSpec::offline("b").backend(backend_b.addr())))
        .plugin(sse_bridge(events))
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn_a = backend_a.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    sync(&mut session, player.as_ref()).await;
    assert_eq!(switches(&mut received), Vec::<Value>::new());

    player.switch_server(ServerId::new("b")).await.unwrap();
    session.expect_join(T).await.unwrap();
    let _conn_b = backend_b.next_connection(T).await.unwrap();
    sync(&mut session, player.as_ref()).await;

    assert_eq!(
        switches(&mut received),
        [json!({
            "type": "PlayerSwitch",
            "data": {
                "player_id": player.id().as_u64(),
                "username": STEVE,
                "from_server": "a",
                "to_server": "b",
                "timestamp": "-",
            },
        })]
    );

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(the_admin_api_reports_a_switch_but_not_a_first_join; p47 = 47, p764 = 764, p774 = 774);

fn sleeping_manager(workdir: &std::path::Path) -> impl FnOnce(&mut Table) + Send + 'static {
    let workdir = workdir.to_str().unwrap().to_string();
    move |table: &mut Table| {
        let manager: Table = toml::from_str(&format!(
            "type = \"local\"\ncommand = \"sleep\"\nargs = [\"10\"]\nworking_dir = {workdir:?}\n"
        ))
        .unwrap();
        table.insert("server_manager".into(), toml::Value::Table(manager));
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_wake_holds_the_player_until_the_server_is_online() {
    let version = ProtocolVersion(CURRENT);
    let workdir = tempfile::tempdir().unwrap();
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let lobby_first = ScriptedPlugin::new("lobby_first")
        .on::<PlayerChooseInitialServerEvent>(EventPriority::NORMAL, |e| {
            e.redirect_to(ServerId::new("lobby"))
        });
    let proxy = TestProxy::builder()
        .server(network(ServerSpec::offline("hub").unreachable()))
        .server(network(
            ServerSpec::offline("lobby")
                .backend(backend.addr())
                .limbo_handlers(["server_wake"])
                .patch(sleeping_manager(workdir.path())),
        ))
        .plugin(lobby_first)
        .plugin(ServerWakePlugin::new())
        .plugin(recorder.plugin())
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
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    let registry = &proxy.services().connection_registry;
    recorder
        .wait_for(
            |e| {
                e.kind == EventKind::ServerStateChange && e.detail["new_state"] == json!("starting")
            },
            T,
        )
        .await
        .unwrap();
    assert_eq!(player.current_server(), None);
    assert_eq!(registry.count_by_server("lobby"), 1);
    assert_eq!(registry.count_by_server("hub"), 0);
    assert_eq!(backend.accepted_connections(), 0);

    proxy
        .bus()
        .fire(ServerStateChangeEvent {
            server: ServerId::new("lobby"),
            old_state: ServerState::Starting,
            new_state: ServerState::Online,
        })
        .await;
    let _conn = backend.next_connection(T).await.unwrap();
    session.expect_join(T).await.unwrap();
    sync(&mut session, player.as_ref()).await;

    let pre_connects = recorder.of(EventKind::ServerPreConnect);
    assert_eq!(pre_connects.len(), 1, "{pre_connects:?}");
    assert_eq!(pre_connects[0].detail["server"], json!("lobby"));
    let joined = recorder.of(EventKind::ServerPostConnect);
    assert_eq!(joined.len(), 1, "{joined:?}");
    assert_eq!(
        joined[0].detail,
        json!({ "server": "lobby", "previous_server": null, "current_server": "lobby" })
    );
    assert_eq!(player.current_server(), Some(ServerId::new("lobby")));
    assert_eq!(registry.count_by_server("lobby"), 1);

    session.quit().await;
    proxy.shutdown().await.unwrap();
}
