#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_api::limbo::handler::LimboOutcome;
use infrarust_api::types::ServerId;
use infrarust_core::test_support::join_game_frame;
use infrarust_protocol::packets::login::ProfileKey;
use infrarust_protocol::packets::play::chat_session::SChatSessionUpdate;
use infrarust_protocol::packets::play::join_game::CJoinGame;
use infrarust_protocol::packets::play::keepalive::SKeepAlive;
use infrarust_test_harness::{
    BackendConn, ClientSession, DEFAULT_TIMEOUT, FakeBackend, FakeSessionServer, PacketFrame,
    ProtocolVersion, ServerSpec, TestProxy, holding_gate, next_hold, wire,
};

const T: Duration = DEFAULT_TIMEOUT;
const VERSION: ProtocolVersion = ProtocolVersion::CURRENT;
const BEFORE_THE_FLAG: ProtocolVersion = ProtocolVersion::V26_1;
const GATE: &str = "gate";
const STEVE: &str = "Steve";
const MARKER: i64 = 4242;

fn online_mode(join: &PacketFrame) -> bool {
    wire::decode::<CJoinGame>(join, VERSION)
        .unwrap()
        .online_mode
}

fn identity_forwarded(spec: ServerSpec) -> ServerSpec {
    spec.patch(|table| {
        table.insert(
            "forwarding_mode".into(),
            toml::Value::String("legacy".into()),
        );
    })
}

fn chat_session() -> SChatSessionUpdate {
    SChatSessionUpdate {
        session_id: uuid::Uuid::from_u128(7),
        profile_key: ProfileKey {
            expires_at: i64::MAX,
            public_key: vec![1; 16],
            key_signature: vec![2; 32],
        },
    }
}

async fn login(proxy: &TestProxy, entry: &str, version: ProtocolVersion) -> ClientSession {
    proxy
        .client_for(entry, version)
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap()
}

async fn chat_sessions_before_the_marker(
    session: &ClientSession,
    conn: &mut BackendConn,
) -> Vec<SChatSessionUpdate> {
    session.send_packet(&chat_session()).await.unwrap();
    session
        .send_packet(&SKeepAlive { id: MARKER })
        .await
        .unwrap();
    let mut seen = Vec::new();
    loop {
        let frame = conn.recv_frame(T).await.unwrap();
        if wire::is::<SChatSessionUpdate>(&frame, VERSION) {
            seen.push(wire::decode(&frame, VERSION).unwrap());
        } else if wire::is::<SKeepAlive>(&frame, VERSION)
            && wire::decode::<SKeepAlive>(&frame, VERSION).unwrap().id == MARKER
        {
            return seen;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_verified_player_is_told_online_mode_by_limbo_backend_and_switch() {
    let compressed = || FakeBackend::builder().compression(Some(256)).spawn();
    let first = compressed().await.unwrap();
    let second = compressed().await.unwrap();
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let (gate, mut holds) = holding_gate("gatekeeper", GATE);
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::client_only("first")
                .backend(first.addr())
                .network("main")
                .limbo_handlers([GATE]),
        )
        .server(
            ServerSpec::client_only("second")
                .backend(second.addr())
                .network("main"),
        )
        .session_server(&sessions)
        .plugin(gate)
        .start()
        .await
        .unwrap();
    assert!(!online_mode(&join_game_frame(VERSION).unwrap()));

    let mut session = login(&proxy, "first", VERSION).await;
    assert!(online_mode(session.join_frame()), "limbo");

    let handle = next_hold(&mut holds, T).await.unwrap().handle;
    handle.complete(LimboOutcome::Accept);
    assert!(
        online_mode(&session.expect_join(T).await.unwrap()),
        "backend"
    );
    let _first = first.next_connection(T).await.unwrap();

    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    player.switch_server(ServerId::new("second")).await.unwrap();
    assert!(
        online_mode(&session.expect_join(T).await.unwrap()),
        "switch"
    );

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_verified_player_joining_a_backend_directly_is_told_online_mode() {
    for compression in [None, Some(256), Some(0)] {
        let backend = FakeBackend::builder()
            .compression(compression)
            .spawn()
            .await
            .unwrap();
        let sessions = FakeSessionServer::spawn().await.unwrap();
        let proxy = TestProxy::builder()
            .server(ServerSpec::client_only("lobby").backend(backend.addr()))
            .session_server(&sessions)
            .start()
            .await
            .unwrap();

        let mut session = login(&proxy, "lobby", VERSION).await;
        assert!(online_mode(session.join_frame()), "join {compression:?}");

        let mut conn = backend.next_connection(T).await.unwrap();
        conn.reconfigure(T).await.unwrap();
        conn.finish_config(T).await.unwrap();
        assert!(
            online_mode(&session.expect_join(T).await.unwrap()),
            "reconfiguration {compression:?}"
        );

        proxy.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_verified_player_gets_the_backends_join_game_as_is_before_26_2() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("lobby").backend(backend.addr()))
        .session_server(&sessions)
        .start()
        .await
        .unwrap();

    let session = login(&proxy, "lobby", BEFORE_THE_FLAG).await;

    assert_eq!(
        session.join_frame().payload,
        join_game_frame(BEFORE_THE_FLAG).unwrap().payload
    );

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_offline_player_gets_the_backends_join_game_as_is() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let session = login(&proxy, "lobby", VERSION).await;

    assert_eq!(
        session.join_frame().payload,
        join_game_frame(VERSION).unwrap().payload
    );

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_chat_session_reaches_only_a_backend_that_knows_the_verified_identity() {
    let forwarded = FakeBackend::builder().spawn().await.unwrap();
    let plain = FakeBackend::builder().spawn().await.unwrap();
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(identity_forwarded(
            ServerSpec::client_only("forwarded")
                .backend(forwarded.addr())
                .network("main"),
        ))
        .server(
            ServerSpec::client_only("plain")
                .backend(plain.addr())
                .network("main"),
        )
        .session_server(&sessions)
        .start()
        .await
        .unwrap();

    let mut session = login(&proxy, "forwarded", VERSION).await;
    let mut conn = forwarded.next_connection(T).await.unwrap();
    assert_eq!(
        chat_sessions_before_the_marker(&session, &mut conn).await,
        [chat_session()]
    );

    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    player.switch_server(ServerId::new("plain")).await.unwrap();
    session.expect_join(T).await.unwrap();
    let mut conn = plain.next_connection(T).await.unwrap();
    assert!(
        chat_sessions_before_the_marker(&session, &mut conn)
            .await
            .is_empty()
    );

    player
        .switch_server(ServerId::new("forwarded"))
        .await
        .unwrap();
    session.expect_join(T).await.unwrap();
    let mut conn = forwarded.next_connection(T).await.unwrap();
    assert_eq!(
        chat_sessions_before_the_marker(&session, &mut conn).await,
        [chat_session()]
    );

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_offline_players_chat_session_never_reaches_a_forwarding_backend() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(identity_forwarded(
            ServerSpec::offline("lobby").backend(backend.addr()),
        ))
        .start()
        .await
        .unwrap();

    let session = login(&proxy, "lobby", VERSION).await;
    let mut conn = backend.next_connection(T).await.unwrap();

    assert!(
        chat_sessions_before_the_marker(&session, &mut conn)
            .await
            .is_empty()
    );

    proxy.shutdown().await.unwrap();
}
