#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use infrarust_api::types::ServerId;
use infrarust_core::hot_restart::resume::RESUME_TOKEN_LEN;
use infrarust_core::hot_restart::{InheritanceOutcome, RESUME_COOKIE};
use infrarust_core::player::{RESTART_REASON, SHUTDOWN_REASON};
use infrarust_core::test_support::FakePreflight;
use infrarust_protocol::packets::play::start_configuration::SAcknowledgeConfiguration;
use infrarust_test_harness::{
    ClientSession, CookieJar, DEFAULT_TIMEOUT, EventKind, FakeBackend, FakeSessionServer,
    LOGIN_INTENT, ProtocolVersion, ProxyRun, Recorder, ServerSpec, TRANSFER_INTENT, TestProxy,
    TestProxyBuilder, version_matrix, wait_until,
};

const T: Duration = DEFAULT_TIMEOUT;
const RESTART: Duration = Duration::from_secs(20);
const MODERN: ProtocolVersion = ProtocolVersion(774);
const RATE_LIMITED: &str = "Rate limit exceeded";
const NO_COOKIE: [&str; 0] = [];

fn requested(proxy: TestProxy) -> ProxyRun {
    let run = proxy.run_to_exit().unwrap();
    run.hub()
        .expect("the proxy runs with hot restart")
        .request()
        .unwrap();
    run
}

async fn joined(
    proxy: &TestProxy,
    server: &str,
    version: ProtocolVersion,
    name: &str,
) -> ClientSession {
    proxy
        .client_for(server, version)
        .unwrap()
        .login(name)
        .await
        .unwrap()
        .joined()
        .unwrap()
}

async fn came_back(
    proxy: &TestProxy,
    server: &str,
    version: ProtocolVersion,
    name: &str,
    jar: CookieJar,
) -> ClientSession {
    proxy
        .client_for(server, version)
        .unwrap()
        .come_back(name, jar)
        .await
        .unwrap()
}

async fn offline_lobby_with(
    build: impl FnOnce(TestProxyBuilder) -> TestProxyBuilder,
) -> (FakeBackend, TestProxy) {
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let builder = TestProxy::builder().server(ServerSpec::offline("lobby").backend(lobby.addr()));
    let proxy = build(builder.hot_restart()).start().await.unwrap();
    (lobby, proxy)
}

async fn offline_lobby() -> (FakeBackend, TestProxy) {
    offline_lobby_with(|builder| builder).await
}

async fn online_vault(
    build: impl FnOnce(TestProxyBuilder) -> TestProxyBuilder,
) -> (FakeSessionServer, FakeBackend, TestProxy) {
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let vault = FakeBackend::builder().spawn().await.unwrap();
    let builder = TestProxy::builder()
        .server(ServerSpec::client_only("vault").backend(vault.addr()))
        .session_server(&sessions);
    let proxy = build(builder.hot_restart()).start().await.unwrap();
    (sessions, vault, proxy)
}

async fn a_restart_tells_the_player_to_reconnect_and_issues_no_ticket(
    build: impl FnOnce(TestProxyBuilder) -> TestProxyBuilder,
    version: ProtocolVersion,
) {
    let (lobby, proxy) = offline_lobby_with(build).await;
    let mut session = joined(&proxy, "lobby", version, "Notch").await;
    let _backend = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let kicked = session.expect_disconnect(T).await.unwrap();
    assert_eq!(kicked.text, RESTART_REASON, "{kicked:?}");
    let successor = run.handed_over(RESTART).await.unwrap();
    assert!(successor.handover().package.state.resumes.is_empty());
}

async fn a_modern_offline_player_comes_back_on_its_own(version: ProtocolVersion) {
    let (lobby, proxy) = offline_lobby().await;
    let addr = proxy.addr();
    let domain = proxy.domain("lobby").unwrap().to_string();
    let session = joined(&proxy, "lobby", version, "Notch").await;
    let first = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let back = session.sent_back(T).await.unwrap();
    assert_eq!(
        (back.host.as_str(), back.port),
        (domain.as_str(), i32::from(addr.port()))
    );
    assert_eq!(back.jar.requested(), NO_COOKIE);
    first.closed(T).await.unwrap();
    let successor = run.handed_over(RESTART).await.unwrap();
    assert_eq!(successor.handover().package.state.resumes.len(), 1);
    assert!(successor.handover().package.state.sessions.is_empty());

    let after = Recorder::new();
    let proxy = successor
        .builder()
        .plugin(after.plugin())
        .start()
        .await
        .unwrap();
    assert_eq!(
        proxy.inheritance().unwrap().outcome(),
        InheritanceOutcome::Committed
    );
    let session = came_back(&proxy, "lobby", version, "Notch", back.jar).await;
    assert_eq!(session.cookies().requested(), [RESUME_COOKIE]);
    let second = lobby.next_connection(T).await.unwrap();
    assert_eq!(second.handshake().next_state, LOGIN_INTENT);
    assert_eq!(second.username(), "Notch");
    proxy.wait_for_player("Notch", T).await.unwrap();
    assert_eq!(after.count(EventKind::PostLogin), 1);
    assert!(!proxy.services().resumes.expects_any());
    proxy.shutdown().await.unwrap();
}

version_matrix!(a_modern_offline_player_comes_back_on_its_own;
    p766 = 766, p769 = 769, p774 = 774, p777 = 777);

async fn an_older_client_is_still_told_to_reconnect(version: ProtocolVersion) {
    a_restart_tells_the_player_to_reconnect_and_issues_no_ticket(|builder| builder, version).await;
}

version_matrix!(an_older_client_is_still_told_to_reconnect; p47 = 47, p764 = 764, p765 = 765);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_online_player_resumes_without_a_second_session_check() {
    let (sessions, vault, proxy) = online_vault(|builder| builder).await;
    let session = joined(&proxy, "vault", MODERN, "Notch").await;
    let first = vault.next_connection(T).await.unwrap();
    let uuid = proxy
        .wait_for_player("Notch", T)
        .await
        .unwrap()
        .profile()
        .uuid;
    assert_eq!(sessions.calls().len(), 1);

    let run = requested(proxy);
    let back = session.sent_back(T).await.unwrap();
    let successor = run.handed_over(RESTART).await.unwrap();
    let record = &successor.handover().package.state.resumes[0];
    assert!(record.online_mode);
    assert_eq!(record.profile.uuid, uuid);

    let proxy = successor.builder().start().await.unwrap();
    let resumed = came_back(&proxy, "vault", MODERN, "Notch", back.jar.clone()).await;
    assert!(
        resumed.server_hash().is_some(),
        "the resumed connection must still be encrypted"
    );
    let second = vault.next_connection(T).await.unwrap();
    assert_eq!(second.handshake().next_state, LOGIN_INTENT);
    assert_eq!(second.uuid(), first.uuid());
    let player = proxy.wait_for_player("Notch", T).await.unwrap();
    assert_eq!(player.profile().uuid, uuid);
    assert!(player.is_online_mode());
    assert_eq!(sessions.calls().len(), 1, "{:?}", sessions.calls());
    resumed.quit().await;
    proxy.wait_for_connection_count(0, T).await.unwrap();

    let _replayed = came_back(&proxy, "vault", MODERN, "Notch", back.jar).await;
    assert_eq!(
        sessions.calls().len(),
        2,
        "a ticket that was already used must not skip the session check"
    );
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ticket_does_not_resume_another_player() {
    let (sessions, vault, proxy) = online_vault(|builder| builder).await;
    let session = joined(&proxy, "vault", MODERN, "Notch").await;
    let _first = vault.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let back = session.sent_back(T).await.unwrap();
    let successor = run.handed_over(RESTART).await.unwrap();
    let proxy = successor.builder().start().await.unwrap();

    let _thief = came_back(&proxy, "vault", MODERN, "Herobrine", back.jar).await;
    let calls = sessions.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[1].0, "Herobrine");
    let backend = vault.next_connection(T).await.unwrap();
    assert_eq!(backend.username(), "Herobrine");
    assert_eq!(backend.handshake().next_state, LOGIN_INTENT);
    assert!(
        !proxy.services().resumes.expects_any(),
        "a ticket presented by someone else is burnt"
    );
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_forged_ticket_falls_back_to_the_normal_login() {
    let (sessions, vault, proxy) = online_vault(|builder| builder).await;
    let session = joined(&proxy, "vault", MODERN, "Notch").await;
    let _first = vault.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let back = session.sent_back(T).await.unwrap();
    let successor = run.handed_over(RESTART).await.unwrap();
    let proxy = successor.builder().start().await.unwrap();

    let forged = CookieJar::new().with(RESUME_COOKIE, vec![7u8; RESUME_TOKEN_LEN]);
    let impostor = came_back(&proxy, "vault", MODERN, "Notch", forged.clone()).await;
    assert_eq!(forged.requested(), [RESUME_COOKIE]);
    assert_eq!(sessions.calls().len(), 2);
    let checked = vault.next_connection(T).await.unwrap();
    assert_eq!(checked.handshake().next_state, LOGIN_INTENT);
    assert!(
        proxy.services().resumes.expects_any(),
        "a forged ticket must not burn the real one"
    );
    impostor.quit().await;
    checked.closed(T).await.unwrap();
    proxy.wait_for_connection_count(0, T).await.unwrap();

    let _resumed = came_back(&proxy, "vault", MODERN, "Notch", back.jar).await;
    let again = vault.next_connection(T).await.unwrap();
    assert_eq!(again.handshake().next_state, LOGIN_INTENT);
    assert_eq!(
        sessions.calls().len(),
        2,
        "the real ticket must still resume its player without a session check: {:?}",
        sessions.calls()
    );
    assert!(!proxy.services().resumes.expects_any());
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_transfer_without_a_ticket_is_an_ordinary_transfer() {
    let (lobby, proxy) = offline_lobby().await;
    let session = joined(&proxy, "lobby", MODERN, "Notch").await;
    let _first = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let _back = session.sent_back(T).await.unwrap();
    let successor = run.handed_over(RESTART).await.unwrap();
    let proxy = successor.builder().start().await.unwrap();

    let _stranger = came_back(&proxy, "lobby", MODERN, "Alex", CookieJar::new()).await;
    let backend = lobby.next_connection(T).await.unwrap();
    assert_eq!(backend.username(), "Alex");
    assert_eq!(backend.handshake().next_state, TRANSFER_INTENT);
    assert!(proxy.services().resumes.expects_any());
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_proxy_that_expects_nobody_asks_for_no_cookie() {
    let (lobby, proxy) = offline_lobby().await;
    let jar = CookieJar::new().with(RESUME_COOKIE, vec![7u8; RESUME_TOKEN_LEN]);
    let _session = came_back(&proxy, "lobby", MODERN, "Notch", jar.clone()).await;
    assert_eq!(
        jar.requested(),
        NO_COOKIE,
        "a proxy that waits for nobody must not ask for the cookie"
    );
    let backend = lobby.next_connection(T).await.unwrap();
    assert_eq!(backend.handshake().next_state, TRANSFER_INTENT);
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_successor_that_cannot_resume_gets_no_player_sent_back() {
    a_restart_tells_the_player_to_reconnect_and_issues_no_ticket(
        |builder| builder.hot_restart_with(FakePreflight::approving_without_resume()),
        MODERN,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transfers_can_be_turned_off() {
    a_restart_tells_the_player_to_reconnect_and_issues_no_ticket(
        |builder| builder.config_section("restart", "transfer = false"),
        MODERN,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reauthentication_can_be_required_on_resume() {
    let (sessions, vault, proxy) =
        online_vault(|builder| builder.config_section("restart", "reauthenticate = true")).await;
    let session = joined(&proxy, "vault", MODERN, "Notch").await;
    let _first = vault.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let back = session.sent_back(T).await.unwrap();
    let successor = run.handed_over(RESTART).await.unwrap();
    let proxy = successor.builder().start().await.unwrap();
    let _session = came_back(&proxy, "vault", MODERN, "Notch", back.jar).await;
    assert_eq!(sessions.calls().len(), 2);
    let backend = vault.next_connection(T).await.unwrap();
    assert_eq!(backend.handshake().next_state, LOGIN_INTENT);
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_player_comes_back_to_the_server_it_had_switched_to() {
    let a = FakeBackend::builder().spawn().await.unwrap();
    let b = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .offline_pair(a.addr(), b.addr())
        .hot_restart()
        .start()
        .await
        .unwrap();
    let session = joined(&proxy, "a", MODERN, "Notch").await;
    let _on_a = a.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player("Notch", T).await.unwrap();
    player.switch_server(ServerId::new("b")).await.unwrap();
    let _on_b = b.next_connection(T).await.unwrap();
    wait_until("the switch to settle", T, || {
        player.current_server() == Some(ServerId::new("b"))
    })
    .await
    .unwrap();
    drop(player);

    let run = requested(proxy);
    let back = session.sent_back(T).await.unwrap();
    let successor = run.handed_over(RESTART).await.unwrap();
    assert_eq!(
        successor.handover().package.state.resumes[0]
            .server
            .as_deref(),
        Some("b")
    );
    let proxy = successor.builder().start().await.unwrap();
    let before = a.accepted_connections();
    let _session = came_back(&proxy, "a", MODERN, "Notch", back.jar).await;
    let again = b.next_connection(T).await.unwrap();
    assert_eq!(again.username(), "Notch");
    assert_eq!(again.handshake().next_state, LOGIN_INTENT);
    assert_eq!(a.accepted_connections(), before);
    let player = proxy.wait_for_player("Notch", T).await.unwrap();
    assert_eq!(player.current_server(), Some(ServerId::new("b")));
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_player_still_configuring_is_sent_back_with_configuration_packets() {
    let (lobby, proxy) = offline_lobby().await;
    let session = joined(&proxy, "lobby", MODERN, "Notch").await;
    let mut backend = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();
    backend.start_configuration().await.unwrap();
    backend
        .expect::<SAcknowledgeConfiguration>(T)
        .await
        .unwrap();
    let addr = proxy.addr();
    let domain = proxy.domain("lobby").unwrap().to_string();

    let run = requested(proxy);
    let back = session.sent_back_while_configuring(T).await.unwrap();
    assert_eq!(
        (back.host.as_str(), back.port),
        (domain.as_str(), i32::from(addr.port()))
    );
    let successor = run.handed_over(RESTART).await.unwrap();
    assert_eq!(successor.handover().package.state.resumes.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stopped_proxy_sends_nobody_back() {
    let (lobby, proxy) = offline_lobby().await;
    let mut session = joined(&proxy, "lobby", MODERN, "Notch").await;
    let _backend = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();
    let services = proxy.services().clone();

    let stopping = tokio::spawn(proxy.shutdown());
    let kicked = session.expect_disconnect(T).await.unwrap();
    assert_eq!(kicked.text, SHUTDOWN_REASON, "{kicked:?}");
    stopping.await.unwrap().unwrap();
    assert!(services.resumes.take_issued().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn players_sent_back_are_not_held_by_the_login_rate_limit() {
    let (lobby, proxy) = offline_lobby_with(|builder| {
        builder.config_section(
            "rate_limit",
            "enabled = true\nmax_connections = 2\nwindow = \"10m\"",
        )
    })
    .await;
    let mut sessions = Vec::new();
    for name in ["Notch", "Jeb"] {
        let session = joined(&proxy, "lobby", MODERN, name).await;
        let backend = lobby.next_connection(T).await.unwrap();
        proxy.wait_for_player(name, T).await.unwrap();
        sessions.push((name, session, backend));
    }

    let run = requested(proxy);
    let mut tickets = Vec::new();
    for (name, session, _backend) in sessions {
        tickets.push((name, session.sent_back(T).await.unwrap().jar));
    }
    let successor = run.handed_over(RESTART).await.unwrap();
    let proxy = successor.builder().start().await.unwrap();

    for name in ["Alex", "Steve"] {
        joined(&proxy, "lobby", MODERN, name).await.quit().await;
    }
    let late = proxy
        .client_for("lobby", MODERN)
        .unwrap()
        .login("Late")
        .await
        .unwrap()
        .disconnected()
        .expect("the login quota must be spent before the players come back");
    assert_eq!(late.text, RATE_LIMITED, "{late:?}");
    let spent = lobby.accepted_connections();
    let mut kept = Vec::new();

    for (name, jar) in tickets {
        let session = proxy
            .client_for("lobby", MODERN)
            .unwrap()
            .come_back(name, jar)
            .await
            .unwrap_or_else(|error| panic!("{name} was held by the rate limit: {error}"));
        kept.push((session, lobby.next_connection(T).await.unwrap()));
        proxy.wait_for_player(name, T).await.unwrap();
    }
    assert_eq!(lobby.accepted_connections(), spent + 2);

    let stranger = proxy
        .client_for("lobby", MODERN)
        .unwrap()
        .transferred()
        .login("Stranger")
        .await
        .unwrap()
        .disconnected()
        .expect("a transfer without a waiting ticket is still rate limited");
    assert_eq!(stranger.text, RATE_LIMITED, "{stranger:?}");
    assert_eq!(lobby.accepted_connections(), spent + 2);
    proxy.shutdown().await.unwrap();
}
