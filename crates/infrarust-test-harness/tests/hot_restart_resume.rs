#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use infrarust_api::types::ServerId;
use infrarust_core::hot_restart::{InheritanceOutcome, RESUME_COOKIE};
use infrarust_core::player::RESTART_REASON;
use infrarust_core::test_support::FakePreflight;
use infrarust_protocol::packets::cookie::{CConfigStoreCookie, CStoreCookie};
use infrarust_protocol::packets::play::start_configuration::SAcknowledgeConfiguration;
use infrarust_protocol::packets::play::transfer::{CConfigTransfer, CTransfer};
use infrarust_test_harness::{
    ClientSession, CookieJar, DEFAULT_TIMEOUT, EventKind, FakeBackend, FakeSessionServer,
    LoginOutcome, ProtocolVersion, ProxyEnd, ProxyRun, Recorder, ServerSpec, Successor, TestProxy,
    version_matrix,
};

const T: Duration = DEFAULT_TIMEOUT;
const RESTART: Duration = Duration::from_secs(20);
const MODERN: ProtocolVersion = ProtocolVersion(774);
const LOGIN: i32 = 2;
const TRANSFER: i32 = 3;

struct SentBack {
    jar: CookieJar,
    host: String,
    port: i32,
}

fn requested(proxy: TestProxy) -> ProxyRun {
    let run = proxy.run_to_exit().unwrap();
    run.hub()
        .expect("the proxy runs with hot restart")
        .request()
        .unwrap();
    run
}

async fn sent_back(mut session: ClientSession) -> SentBack {
    let cookie = session.expect::<CStoreCookie>(T).await.unwrap();
    assert_eq!(cookie.key, RESUME_COOKIE);
    assert_eq!(cookie.payload.len(), 32);
    let transfer = session.expect::<CTransfer>(T).await.unwrap();
    let jar = session.cookies();
    assert_eq!(jar.get(RESUME_COOKIE), Some(cookie.payload));
    session.quit().await;
    SentBack {
        jar,
        host: transfer.host,
        port: transfer.port,
    }
}

async fn handed(run: ProxyRun) -> Successor {
    match run.finish(RESTART).await.unwrap() {
        ProxyEnd::Restarted(successor) => *successor,
        ProxyEnd::Stopped => panic!("the restart must hand over"),
    }
}

async fn joined(proxy: &TestProxy, server: &str, version: ProtocolVersion) -> ClientSession {
    proxy
        .client_for(server, version)
        .unwrap()
        .login("Notch")
        .await
        .unwrap()
        .joined()
        .unwrap()
}

async fn offline_lobby() -> (FakeBackend, TestProxy) {
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(lobby.addr()))
        .hot_restart()
        .start()
        .await
        .unwrap();
    (lobby, proxy)
}

async fn online_vault(
    patch: impl FnOnce(&mut toml::Table) + Send + 'static,
) -> (FakeSessionServer, FakeBackend, TestProxy) {
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let vault = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("vault").backend(vault.addr()))
        .session_server(&sessions)
        .patch_config(patch)
        .hot_restart()
        .start()
        .await
        .unwrap();
    (sessions, vault, proxy)
}

async fn a_modern_offline_player_comes_back_on_its_own(version: ProtocolVersion) {
    let (lobby, proxy) = offline_lobby().await;
    let addr = proxy.addr();
    let domain = proxy.domain("lobby").unwrap().to_string();
    let session = joined(&proxy, "lobby", version).await;
    let first = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let back = sent_back(session).await;
    assert_eq!(
        (back.host.as_str(), back.port),
        (domain.as_str(), i32::from(addr.port()))
    );
    first.closed(T).await.unwrap();
    let successor = handed(run).await;
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
    let _session = proxy
        .client_for("lobby", version)
        .unwrap()
        .transferred()
        .cookies(back.jar)
        .login("Notch")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let second = lobby.next_connection(T).await.unwrap();
    assert_eq!(second.handshake().next_state, LOGIN);
    assert_eq!(second.username(), "Notch");
    proxy.wait_for_player("Notch", T).await.unwrap();
    assert_eq!(after.count(EventKind::PostLogin), 1);
    assert!(!proxy.services().resumes.expects_any());
    proxy.shutdown().await.unwrap();
}

version_matrix!(a_modern_offline_player_comes_back_on_its_own;
    p766 = 766, p769 = 769, p774 = 774, p777 = 777);

async fn an_older_client_is_still_told_to_reconnect(version: ProtocolVersion) {
    let (lobby, proxy) = offline_lobby().await;
    let mut session = joined(&proxy, "lobby", version).await;
    let _backend = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let kicked = session.expect_disconnect(T).await.unwrap();
    assert_eq!(kicked.text, RESTART_REASON, "{kicked:?}");
    let successor = handed(run).await;
    assert!(successor.handover().package.state.resumes.is_empty());
}

version_matrix!(an_older_client_is_still_told_to_reconnect; p47 = 47, p764 = 764, p765 = 765);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_online_player_resumes_without_a_second_session_check() {
    let (sessions, vault, proxy) = online_vault(|_| {}).await;
    let session = joined(&proxy, "vault", MODERN).await;
    let first = vault.next_connection(T).await.unwrap();
    let uuid = proxy
        .wait_for_player("Notch", T)
        .await
        .unwrap()
        .profile()
        .uuid;
    assert_eq!(sessions.calls().len(), 1);

    let run = requested(proxy);
    let back = sent_back(session).await;
    let successor = handed(run).await;
    let record = &successor.handover().package.state.resumes[0];
    assert!(record.online_mode);
    assert_eq!(record.profile.uuid, uuid);

    let proxy = successor.builder().start().await.unwrap();
    let resumed = proxy
        .client_for("vault", MODERN)
        .unwrap()
        .transferred()
        .cookies(back.jar.clone())
        .login("Notch")
        .await
        .unwrap()
        .joined()
        .unwrap();
    assert!(
        resumed.server_hash().is_some(),
        "the resumed connection must still be encrypted"
    );
    let second = vault.next_connection(T).await.unwrap();
    assert_eq!(second.handshake().next_state, LOGIN);
    assert_eq!(second.uuid(), first.uuid());
    let player = proxy.wait_for_player("Notch", T).await.unwrap();
    assert_eq!(player.profile().uuid, uuid);
    assert!(player.is_online_mode());
    assert_eq!(sessions.calls().len(), 1, "{:?}", sessions.calls());
    resumed.quit().await;
    proxy.wait_for_connection_count(0, T).await.unwrap();

    let replayed = proxy
        .client_for("vault", MODERN)
        .unwrap()
        .transferred()
        .cookies(back.jar)
        .login("Notch")
        .await
        .unwrap()
        .joined()
        .unwrap();
    assert_eq!(
        sessions.calls().len(),
        2,
        "a ticket that was already used must not skip the session check"
    );
    drop(replayed);
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ticket_does_not_resume_another_player() {
    let (sessions, vault, proxy) = online_vault(|_| {}).await;
    let session = joined(&proxy, "vault", MODERN).await;
    let _first = vault.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let back = sent_back(session).await;
    let proxy = handed(run).await.builder().start().await.unwrap();

    let _thief = proxy
        .client_for("vault", MODERN)
        .unwrap()
        .transferred()
        .cookies(back.jar.clone())
        .login("Herobrine")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let calls = sessions.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[1].0, "Herobrine");
    let backend = vault.next_connection(T).await.unwrap();
    assert_eq!(backend.username(), "Herobrine");
    assert_eq!(backend.handshake().next_state, LOGIN);
    assert!(
        !proxy.services().resumes.expects_any(),
        "a ticket presented by someone else is burnt"
    );
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_forged_ticket_falls_back_to_the_normal_login() {
    let (sessions, vault, proxy) = online_vault(|_| {}).await;
    let session = joined(&proxy, "vault", MODERN).await;
    let _first = vault.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let back = sent_back(session).await;
    let proxy = handed(run).await.builder().start().await.unwrap();

    let forged = CookieJar::new().with(RESUME_COOKIE, vec![7u8; 32]);
    let _session = proxy
        .client_for("vault", MODERN)
        .unwrap()
        .transferred()
        .cookies(forged)
        .login("Notch")
        .await
        .unwrap()
        .joined()
        .unwrap();
    assert_eq!(sessions.calls().len(), 2);
    assert_eq!(
        vault
            .next_connection(T)
            .await
            .unwrap()
            .handshake()
            .next_state,
        LOGIN
    );
    assert!(
        proxy.services().resumes.expects_any(),
        "a forged ticket must not burn the real one"
    );
    assert!(back.jar.get(RESUME_COOKIE).is_some());
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_transfer_without_a_ticket_is_an_ordinary_transfer() {
    let (lobby, proxy) = offline_lobby().await;
    let session = joined(&proxy, "lobby", MODERN).await;
    let _first = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let _back = sent_back(session).await;
    let proxy = handed(run).await.builder().start().await.unwrap();

    let _stranger = proxy
        .client_for("lobby", MODERN)
        .unwrap()
        .transferred()
        .login("Alex")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let backend = lobby.next_connection(T).await.unwrap();
    assert_eq!(backend.username(), "Alex");
    assert_eq!(backend.handshake().next_state, TRANSFER);
    assert!(proxy.services().resumes.expects_any());
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_proxy_that_expects_nobody_asks_for_no_cookie() {
    let (lobby, proxy) = offline_lobby().await;
    let jar = CookieJar::new().with(RESUME_COOKIE, vec![7u8; 32]);
    let _session = proxy
        .client_for("lobby", MODERN)
        .unwrap()
        .transferred()
        .cookies(jar)
        .login("Notch")
        .await
        .unwrap()
        .joined()
        .unwrap();
    assert_eq!(
        lobby
            .next_connection(T)
            .await
            .unwrap()
            .handshake()
            .next_state,
        TRANSFER
    );
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_successor_that_cannot_resume_gets_no_player_sent_back() {
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(lobby.addr()))
        .hot_restart_with(FakePreflight::approving_without_resume())
        .start()
        .await
        .unwrap();
    let mut session = joined(&proxy, "lobby", MODERN).await;
    let _backend = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let kicked = session.expect_disconnect(T).await.unwrap();
    assert_eq!(kicked.text, RESTART_REASON, "{kicked:?}");
    assert!(
        handed(run)
            .await
            .handover()
            .package
            .state
            .resumes
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transfers_can_be_turned_off() {
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(lobby.addr()))
        .patch_config(|config| {
            let mut restart = toml::Table::new();
            restart.insert("transfer".into(), false.into());
            config.insert("restart".into(), restart.into());
        })
        .hot_restart()
        .start()
        .await
        .unwrap();
    let mut session = joined(&proxy, "lobby", MODERN).await;
    let _backend = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let kicked = session.expect_disconnect(T).await.unwrap();
    assert_eq!(kicked.text, RESTART_REASON, "{kicked:?}");
    assert!(
        handed(run)
            .await
            .handover()
            .package
            .state
            .resumes
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reauthentication_can_be_required_on_resume() {
    let (sessions, vault, proxy) = online_vault(|config| {
        let mut restart = toml::Table::new();
        restart.insert("reauthenticate".into(), true.into());
        config.insert("restart".into(), restart.into());
    })
    .await;
    let session = joined(&proxy, "vault", MODERN).await;
    let _first = vault.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();

    let run = requested(proxy);
    let back = sent_back(session).await;
    let proxy = handed(run).await.builder().start().await.unwrap();
    let _session = proxy
        .client_for("vault", MODERN)
        .unwrap()
        .transferred()
        .cookies(back.jar)
        .login("Notch")
        .await
        .unwrap()
        .joined()
        .unwrap();
    assert_eq!(sessions.calls().len(), 2);
    assert_eq!(
        vault
            .next_connection(T)
            .await
            .unwrap()
            .handshake()
            .next_state,
        LOGIN
    );
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
    let session = joined(&proxy, "a", MODERN).await;
    let _on_a = a.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player("Notch", T).await.unwrap();
    player.switch_server(ServerId::new("b")).await.unwrap();
    let _on_b = b.next_connection(T).await.unwrap();
    tokio::time::timeout(T, async {
        while player.current_server() != Some(ServerId::new("b")) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the switch must settle");
    drop(player);

    let run = requested(proxy);
    let back = sent_back(session).await;
    let successor = handed(run).await;
    assert_eq!(successor.handover().package.state.resumes[0].server, "b");
    let proxy = successor.builder().start().await.unwrap();
    let before = a.accepted_connections();
    let _session = proxy
        .client_for("a", MODERN)
        .unwrap()
        .transferred()
        .cookies(back.jar)
        .login("Notch")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let again = b.next_connection(T).await.unwrap();
    assert_eq!(again.username(), "Notch");
    assert_eq!(again.handshake().next_state, LOGIN);
    assert_eq!(a.accepted_connections(), before);
    let player = proxy.wait_for_player("Notch", T).await.unwrap();
    assert_eq!(player.current_server(), Some(ServerId::new("b")));
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_player_still_configuring_is_sent_back_with_configuration_packets() {
    let (lobby, proxy) = offline_lobby().await;
    let mut session = joined(&proxy, "lobby", MODERN).await;
    let mut backend = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();
    backend.start_configuration().await.unwrap();
    backend
        .expect::<SAcknowledgeConfiguration>(T)
        .await
        .unwrap();
    let port = proxy.addr().port();

    let run = requested(proxy);
    let cookie = session
        .expect_config::<CConfigStoreCookie>(T)
        .await
        .unwrap();
    assert_eq!(cookie.key, RESUME_COOKIE);
    assert_eq!(cookie.payload.len(), 32);
    let transfer = session.expect_config::<CConfigTransfer>(T).await.unwrap();
    assert_eq!(transfer.port, i32::from(port));
    session.quit().await;
    let successor = handed(run).await;
    assert_eq!(successor.handover().package.state.resumes.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stopped_proxy_sends_nobody_back() {
    let (lobby, proxy) = offline_lobby().await;
    let mut session = joined(&proxy, "lobby", MODERN).await;
    let _backend = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();
    let services = Arc::new(proxy.services().clone());

    let stopping = tokio::spawn(proxy.shutdown());
    let kicked = session.expect_disconnect(T).await.unwrap();
    assert_ne!(kicked.text, RESTART_REASON);
    stopping.await.unwrap().unwrap();
    assert!(services.resumes.take_issued().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn players_sent_back_are_not_held_by_the_login_rate_limit() {
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(lobby.addr()))
        .patch_config(|config| {
            let rate_limit: toml::Table =
                toml::from_str("enabled = true\nmax_connections = 2\nwindow = \"10m\"").unwrap();
            config.insert("rate_limit".into(), rate_limit.into());
        })
        .hot_restart()
        .start()
        .await
        .unwrap();
    let mut sessions = Vec::new();
    for name in ["Notch", "Jeb"] {
        let session = proxy
            .client_for("lobby", MODERN)
            .unwrap()
            .login(name)
            .await
            .unwrap()
            .joined()
            .unwrap();
        let backend = lobby.next_connection(T).await.unwrap();
        proxy.wait_for_player(name, T).await.unwrap();
        sessions.push((name, session, backend));
    }

    let run = requested(proxy);
    let mut tickets = Vec::new();
    for (name, session, _backend) in sessions {
        tickets.push((name, sent_back(session).await.jar));
    }
    let proxy = handed(run).await.builder().start().await.unwrap();

    for name in ["Alex", "Steve"] {
        proxy
            .client_for("lobby", MODERN)
            .unwrap()
            .login(name)
            .await
            .unwrap()
            .joined()
            .unwrap()
            .quit()
            .await;
    }
    let late = proxy
        .client_for("lobby", MODERN)
        .unwrap()
        .login("Late")
        .await;
    assert!(
        !matches!(late, Ok(LoginOutcome::Joined(_))),
        "the login quota must be spent before the players come back"
    );
    let spent = lobby.accepted_connections();
    let mut kept = Vec::new();

    for (name, jar) in tickets {
        let session = proxy
            .client_for("lobby", MODERN)
            .unwrap()
            .transferred()
            .cookies(jar)
            .login(name)
            .await
            .unwrap()
            .joined()
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
        .await;
    assert!(
        !matches!(stranger, Ok(LoginOutcome::Joined(_))),
        "a transfer without a waiting ticket is still rate limited"
    );
    assert_eq!(lobby.accepted_connections(), spent + 2);
    proxy.shutdown().await.unwrap();
}
