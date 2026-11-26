#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use infrarust_api::services::proxy_control::{
    RestartCancelError, RestartOptions, RestartRequestError,
};
use infrarust_core::hot_restart::{RestartHub, RestartPhase};
use infrarust_core::player::{RESTART_REASON, SHUTDOWN_REASON};
use infrarust_core::test_support::FakePreflight;
use infrarust_protocol::packets::play::transfer::CTransfer;
use infrarust_test_harness::{
    BackendConn, ClientSession, DEFAULT_TIMEOUT, FakeBackend, FakeClient, ProtocolVersion,
    ProxyEnd, ProxyRun, ServerSpec, TestProxy, TestProxyBuilder, wait_until,
};

const T: Duration = DEFAULT_TIMEOUT;
const RESTART: Duration = Duration::from_secs(20);
const MODERN: ProtocolVersion = ProtocolVersion(774);
const OLDER: ProtocolVersion = ProtocolVersion(765);

fn options(delay: Option<Duration>, message: Option<&str>) -> RestartOptions {
    RestartOptions {
        delay,
        message: message.map(str::to_string),
    }
}

async fn lobby_with(
    build: impl FnOnce(TestProxyBuilder) -> TestProxyBuilder,
    version: ProtocolVersion,
) -> (
    FakeBackend,
    BackendConn,
    ProxyRun,
    Arc<RestartHub>,
    ClientSession,
) {
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let builder = TestProxy::builder().server(ServerSpec::offline("lobby").backend(lobby.addr()));
    let proxy = build(builder.hot_restart()).start().await.unwrap();
    let session = proxy
        .client_for("lobby", version)
        .unwrap()
        .login("Notch")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let backend = lobby.next_connection(T).await.unwrap();
    proxy.wait_for_player("Notch", T).await.unwrap();
    let run = proxy.run_to_exit().unwrap();
    let hub = Arc::clone(run.hub().expect("the proxy runs with hot restart"));
    (lobby, backend, run, hub, session)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_delayed_restart_tells_the_players_then_waits_before_it_starts() {
    let (_lobby, _backend, run, hub, mut session) = lobby_with(|builder| builder, MODERN).await;

    let asked = Instant::now();
    let before = SystemTime::now();
    hub.request_with(options(
        Some(Duration::from_secs(2)),
        Some("&eRestart in {delay} ({seconds} s)"),
    ))
    .unwrap();
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "Restart in 2s (2 s)"
    );
    assert_eq!(hub.phase(), RestartPhase::Announcing);
    let scheduled = hub.scheduled().expect("the restart has a date").at;
    let earliest = before + Duration::from_secs(2);
    let latest = SystemTime::now() + Duration::from_secs(2);
    assert!(
        (earliest..=latest).contains(&scheduled),
        "{scheduled:?} is not between {earliest:?} and {latest:?}"
    );
    assert_eq!(hub.request(), Err(RestartRequestError::InProgress));
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "Restart in 1s (1 s)"
    );

    session.expect::<CTransfer>(T).await.unwrap();
    assert!(
        asked.elapsed() >= Duration::from_secs(2),
        "the players were sent back after {:?}",
        asked.elapsed()
    );
    session.quit().await;
    run.handed_over(RESTART).await.unwrap();
    assert_eq!(hub.scheduled(), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_delay_without_a_message_uses_the_default_notice() {
    let (_lobby, _backend, run, hub, mut session) = lobby_with(|builder| builder, MODERN).await;
    hub.request_with(options(Some(Duration::from_secs(1)), None))
        .unwrap();
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "The proxy restarts in 1s"
    );
    session.expect::<CTransfer>(T).await.unwrap();
    session.quit().await;
    run.handed_over(RESTART).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_without_a_delay_is_said_right_before_the_restart() {
    let (_lobby, _backend, run, hub, mut session) = lobby_with(|builder| builder, MODERN).await;
    hub.request_with(options(None, Some("Maintenance, back in a minute")))
        .unwrap();
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "Maintenance, back in a minute"
    );
    session.expect::<CTransfer>(T).await.unwrap();
    session.quit().await;
    run.handed_over(RESTART).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_older_client_reads_the_notice_before_it_is_told_to_reconnect() {
    let (_lobby, _backend, run, hub, mut session) = lobby_with(|builder| builder, OLDER).await;
    hub.request_with(options(
        Some(Duration::from_secs(1)),
        Some("Restart in {delay}"),
    ))
    .unwrap();
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "Restart in 1s"
    );
    let kicked = session.expect_disconnect(T).await.unwrap();
    assert_eq!(kicked.text, RESTART_REASON, "{kicked:?}");
    run.handed_over(RESTART).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_configuration_gives_the_default_delay_and_message() {
    let (_lobby, _backend, run, hub, mut session) = lobby_with(
        |builder| {
            builder.config_section(
                "restart",
                "delay = \"1s\"\nmessage = \"&cUpdate in {delay}\"",
            )
        },
        MODERN,
    )
    .await;
    let asked = Instant::now();
    hub.request().unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "Update in 1s");
    session.expect::<CTransfer>(T).await.unwrap();
    assert!(asked.elapsed() >= Duration::from_secs(1));
    session.quit().await;
    run.handed_over(RESTART).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_can_ask_for_no_delay_despite_the_configuration() {
    let (_lobby, _backend, run, hub, mut session) = lobby_with(
        |builder| builder.config_section("restart", "delay = \"10m\""),
        MODERN,
    )
    .await;
    hub.request_with(options(Some(Duration::ZERO), None))
        .unwrap();
    session.expect::<CTransfer>(T).await.unwrap();
    session.quit().await;
    run.handed_over(RESTART).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_scheduled_restart_can_be_cancelled_and_asked_again() {
    let (lobby, _backend, run, hub, mut session) = lobby_with(|builder| builder, MODERN).await;
    assert_eq!(hub.cancel(), Err(RestartCancelError::NothingScheduled));

    hub.request_with(options(
        Some(Duration::from_secs(600)),
        Some("Restart in {delay}"),
    ))
    .unwrap();
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "Restart in 10m"
    );
    assert_eq!(hub.cancel(), Ok(()));
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "The restart is cancelled"
    );
    wait_until("the hub to be idle again", T, || {
        hub.phase() == RestartPhase::Idle
    })
    .await
    .unwrap();
    assert_eq!(hub.scheduled(), None);
    assert_eq!(
        hub.last_failure().as_deref(),
        Some("the restart was cancelled")
    );
    assert!(!run.is_finished());

    let _other = FakeClient::new(run.addr(), MODERN)
        .domain("lobby.test")
        .login("Alex")
        .await
        .unwrap()
        .joined()
        .expect("a cancelled restart leaves the proxy accepting players");
    let _other_backend = lobby.next_connection(T).await.unwrap();

    hub.request().unwrap();
    session.expect::<CTransfer>(T).await.unwrap();
    session.quit().await;
    run.handed_over(RESTART).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stop_during_the_notice_is_a_normal_stop() {
    let (_lobby, _backend, run, hub, mut session) = lobby_with(|builder| builder, MODERN).await;
    hub.request_with(options(
        Some(Duration::from_secs(600)),
        Some("Restart in {delay}"),
    ))
    .unwrap();
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "Restart in 10m"
    );
    run.shutdown_token().cancel();
    let kicked = session.expect_disconnect(T).await.unwrap();
    assert_eq!(kicked.text, SHUTDOWN_REASON, "{kicked:?}");
    assert!(matches!(
        run.finish(RESTART).await.unwrap(),
        ProxyEnd::Stopped
    ));
    assert_eq!(hub.request(), Err(RestartRequestError::ShuttingDown));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_binary_that_stops_being_valid_during_the_delay_calls_the_restart_off() {
    let preflight = FakePreflight::approving_times(1);
    let checks = Arc::clone(&preflight);
    let (_lobby, _backend, run, hub, mut session) =
        lobby_with(|builder| builder.hot_restart_with(preflight), MODERN).await;
    hub.request_with(options(
        Some(Duration::from_secs(1)),
        Some("Restart in {delay}"),
    ))
    .unwrap();
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "Restart in 1s"
    );
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "The restart is cancelled"
    );
    wait_until("the hub to be idle again", T, || {
        hub.phase() == RestartPhase::Idle
    })
    .await
    .unwrap();
    assert_eq!(checks.checks(), 2);
    let plan = hub.plan();
    assert_eq!(
        hub.last_failure(),
        Some(format!(
            "{} ({}) cannot take over live sessions; restart the proxy by hand to run it",
            plan.target.display(),
            plan.binary.build.version
        ))
    );
    assert!(!run.is_finished());
    run.shutdown_token().cancel();
    assert!(matches!(
        run.finish(RESTART).await.unwrap(),
        ProxyEnd::Stopped
    ));
}
