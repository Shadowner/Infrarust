#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::player::Player;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_test_harness::{
    BackendConn, ClientSession, DEFAULT_TIMEOUT, EventKind, FakeBackend, FakeSessionServer,
    LoginBehavior, ProtocolVersion, Recorder, ServerSpec, TestProxy,
};
use toml::{Table, Value};
use uuid::Uuid;

const T: Duration = DEFAULT_TIMEOUT;
const VERSION: ProtocolVersion = ProtocolVersion(774);
const REPLACED: &str = "You logged in from another location";

fn claimed_uuids(table: &mut Table) {
    table
        .entry("auth")
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .unwrap()
        .insert("offline_uuid".into(), Value::String("client".into()));
}

struct Mixed {
    proxy: TestProxy,
    recorder: Recorder,
    _sessions: FakeSessionServer,
    verified: FakeBackend,
    offline: FakeBackend,
    _forwarded: FakeBackend,
    conns: Mutex<Vec<BackendConn>>,
}

impl Mixed {
    async fn start() -> Self {
        let sessions = FakeSessionServer::spawn().await.unwrap();
        let verified = FakeBackend::builder().spawn().await.unwrap();
        let offline = FakeBackend::builder().spawn().await.unwrap();
        let forwarded = FakeBackend::builder()
            .login(LoginBehavior::Hang)
            .spawn()
            .await
            .unwrap();
        let recorder = Recorder::new();
        let proxy = TestProxy::builder()
            .server(ServerSpec::client_only("verified").backend(verified.addr()))
            .server(ServerSpec::offline("offline").backend(offline.addr()))
            .server(ServerSpec::passthrough("forwarded").backend(forwarded.addr()))
            .session_server(&sessions)
            .plugin(recorder.plugin())
            .patch_config(claimed_uuids)
            .start()
            .await
            .unwrap();
        Self {
            proxy,
            recorder,
            _sessions: sessions,
            verified,
            offline,
            _forwarded: forwarded,
            conns: Mutex::new(Vec::new()),
        }
    }

    async fn join_verified(&self, name: &str) -> (ClientSession, Arc<dyn Player>) {
        let session = self
            .proxy
            .client_for("verified", VERSION)
            .unwrap()
            .login(name)
            .await
            .unwrap()
            .joined()
            .unwrap();
        let conn = self.verified.next_connection(T).await.unwrap();
        self.conns.lock().unwrap().push(conn);
        let player = self.proxy.wait_for_player(name, T).await.unwrap();
        (session, player)
    }

    async fn join_offline(&self, name: &str, uuid: Uuid) -> ClientSession {
        let session = self
            .proxy
            .client_for("offline", VERSION)
            .unwrap()
            .claimed_uuid(uuid)
            .login(name)
            .await
            .unwrap()
            .joined()
            .unwrap();
        let conn = self.offline.next_connection(T).await.unwrap();
        self.conns.lock().unwrap().push(conn);
        session
    }

    async fn start_forwarded(&self, name: &str) -> tokio::task::JoinHandle<()> {
        let seen = self
            .recorder
            .of(EventKind::ServerPreConnect)
            .last()
            .map(|e| e.seq);
        let client = self.proxy.client_for("forwarded", VERSION).unwrap();
        let name = name.to_string();
        let login = tokio::spawn(async move {
            let _ = client.login(&name).await;
        });
        self.recorder
            .wait_for(
                |e| {
                    e.kind == EventKind::ServerPreConnect
                        && e.server() == Some("forwarded")
                        && Some(e.seq) > seen
                },
                T,
            )
            .await
            .unwrap();
        login
    }

    fn found(&self, name: &str) -> Option<u64> {
        self.proxy
            .services()
            .player_registry
            .get_player(name)
            .map(|p| p.id().as_u64())
    }

    fn found_uuid(&self, uuid: &Uuid) -> Option<u64> {
        self.proxy
            .services()
            .player_registry
            .get_player_by_uuid(uuid)
            .map(|p| p.id().as_u64())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_forwarded_login_leaves_a_verified_player_connected() {
    let world = Mixed::start().await;
    let (mut victim, player) = world.join_verified("Steve").await;

    let login = world.start_forwarded("Steve").await;

    assert_eq!(world.recorder.count(EventKind::Disconnect), 0);
    assert!(player.is_connected());
    assert_eq!(world.proxy.connection_count(), 2);
    assert_eq!(world.found("Steve"), Some(player.id().as_u64()));
    assert_eq!(
        world.found_uuid(&offline_uuid("Steve")),
        Some(player.id().as_u64())
    );
    victim.sync_with(player.as_ref(), T).await.unwrap();

    login.abort();
    world.proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_forwarded_login_leaves_an_offline_player_connected() {
    let world = Mixed::start().await;
    let _victim = world.join_offline("Steve", offline_uuid("Steve")).await;
    let player = world.proxy.wait_for_player("Steve", T).await.unwrap();

    let login = world.start_forwarded("Steve").await;

    assert_eq!(world.recorder.count(EventKind::Disconnect), 0);
    assert!(player.is_connected());
    assert_eq!(world.found("Steve"), Some(player.id().as_u64()));

    login.abort();
    world.proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_forwarded_login_leaves_the_first_in_place() {
    let world = Mixed::start().await;
    let first = world.start_forwarded("Steve").await;
    let player = world.proxy.wait_for_player("Steve", T).await.unwrap();

    let second = world.start_forwarded("Steve").await;

    assert_eq!(world.recorder.count(EventKind::Disconnect), 0);
    assert!(player.is_connected());
    assert_eq!(world.proxy.connection_count(), 2);
    assert_eq!(world.found("Steve"), Some(player.id().as_u64()));

    first.abort();
    second.abort();
    world.proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_verified_login_leaves_a_forwarded_session_alone() {
    let world = Mixed::start().await;
    let login = world.start_forwarded("Steve").await;
    let forwarded = world.proxy.wait_for_player("Steve", T).await.unwrap();

    let (_steve, _) = world.join_verified("Steve").await;

    assert_eq!(world.recorder.count(EventKind::Disconnect), 0);
    assert!(forwarded.is_connected());
    assert_eq!(world.proxy.connection_count(), 2);
    let verified = world.found("Steve").unwrap();
    assert_ne!(verified, forwarded.id().as_u64());
    assert_eq!(world.found_uuid(&offline_uuid("Steve")), Some(verified));

    login.abort();
    world.proxy.shutdown().await.unwrap();
}

async fn an_offline_login_cannot_displace_a_verified_player(claimed_name: &str) {
    let world = Mixed::start().await;
    let (mut victim, player) = world.join_verified("Steve").await;

    let _attacker = world
        .join_offline(claimed_name, offline_uuid("Steve"))
        .await;

    assert_eq!(world.recorder.count(EventKind::Disconnect), 0);
    assert!(player.is_connected());
    assert_eq!(world.proxy.connection_count(), 2);
    assert_eq!(world.found("Steve"), Some(player.id().as_u64()));
    assert_eq!(
        world.found_uuid(&offline_uuid("Steve")),
        Some(player.id().as_u64())
    );
    victim.sync_with(player.as_ref(), T).await.unwrap();

    world.proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_offline_login_of_the_same_name_leaves_a_verified_player_connected() {
    an_offline_login_cannot_displace_a_verified_player("Steve").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_offline_login_claiming_the_uuid_leaves_a_verified_player_connected() {
    an_offline_login_cannot_displace_a_verified_player("Mallory").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_verified_login_replaces_an_offline_session_of_that_uuid() {
    let world = Mixed::start().await;
    let mut offline = world.join_offline("Steve", offline_uuid("Steve")).await;

    let (_steve, _) = world.join_verified("Steve").await;

    let info = offline.expect_disconnect(T).await.unwrap();
    assert_eq!(info.text, REPLACED, "{info:?}");
    assert_eq!(world.recorder.count(EventKind::Disconnect), 1);
    assert_eq!(world.proxy.connection_count(), 1);

    world.proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_verified_login_replaces_the_previous_verified_session() {
    let world = Mixed::start().await;
    let (mut first, _) = world.join_verified("Steve").await;

    let (_second, _) = world.join_verified("Steve").await;

    let info = first.expect_disconnect(T).await.unwrap();
    assert_eq!(info.text, REPLACED, "{info:?}");
    assert_eq!(world.proxy.connection_count(), 1);

    world.proxy.shutdown().await.unwrap();
}
