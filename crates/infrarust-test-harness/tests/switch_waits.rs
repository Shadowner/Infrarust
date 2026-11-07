#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use infrarust_api::error::PlayerError;
use infrarust_api::player::{ConnectionResult, Player};
use infrarust_api::types::{Component, ServerId};
use infrarust_test_harness::{
    BackendConn, ClientSession, DEFAULT_TIMEOUT, FakeBackend, ProtocolVersion, TestProxy,
    version_matrix,
};
use tokio::task::JoinHandle;

const T: Duration = DEFAULT_TIMEOUT;
const FAST: Duration = Duration::from_secs(3);
const USERNAME: &str = "Steve";

struct WaitingSwitch {
    proxy: TestProxy,
    session: ClientSession,
    player: Arc<dyn Player>,
    connect: JoinHandle<Result<ConnectionResult, PlayerError>>,
    _old: BackendConn,
    _new: BackendConn,
    _backends: [FakeBackend; 2],
}

async fn switching_to_a_silent_backend(version: ProtocolVersion) -> WaitingSwitch {
    let old_backend = FakeBackend::builder().spawn().await.unwrap();
    let new_backend = FakeBackend::builder().hold_join().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .offline_pair(old_backend.addr(), new_backend.addr())
        .start()
        .await
        .unwrap();
    let session = proxy
        .client_for("a", version)
        .unwrap()
        .login(USERNAME)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let old = old_backend.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player(USERNAME, T).await.unwrap();

    let switching = Arc::clone(&player);
    let connect = tokio::spawn(async move { switching.connect(ServerId::new("b")).await });
    let new = new_backend.next_connection(T).await.unwrap();

    WaitingSwitch {
        proxy,
        session,
        player,
        connect,
        _old: old,
        _new: new,
        _backends: [old_backend, new_backend],
    }
}

async fn a_client_leaving_during_the_wait_ends_its_session(version: ProtocolVersion) {
    let waiting = switching_to_a_silent_backend(version).await;

    waiting.session.quit().await;

    waiting
        .proxy
        .wait_for_connection_count(0, FAST)
        .await
        .unwrap();
    waiting.connect.abort();
    waiting.proxy.shutdown().await.unwrap();
}

version_matrix!(a_client_leaving_during_the_wait_ends_its_session; p47 = 47, p763 = 763, p764 = 764, p777 = 777);

async fn disconnecting_the_player_during_the_wait_ends_its_session(version: ProtocolVersion) {
    let waiting = switching_to_a_silent_backend(version).await;

    waiting.player.disconnect(Component::text("bye")).await;

    waiting
        .proxy
        .wait_for_connection_count(0, FAST)
        .await
        .unwrap();
    drop(waiting.session);
    waiting.connect.abort();
    waiting.proxy.shutdown().await.unwrap();
}

version_matrix!(disconnecting_the_player_during_the_wait_ends_its_session; p47 = 47, p763 = 763, p764 = 764, p777 = 777);

async fn a_silent_backend_fails_the_switch_and_the_player_stays(version: ProtocolVersion) {
    let waiting = switching_to_a_silent_backend(version).await;

    let started = tokio::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(33), waiting.connect)
        .await
        .expect("the switch outlived its time limit")
        .unwrap()
        .unwrap();

    assert!(matches!(result, ConnectionResult::Failed(_)), "{result:?}");
    assert!(
        started.elapsed() >= Duration::from_secs(25),
        "the switch gave up after {:?}",
        started.elapsed()
    );
    assert!(waiting.player.is_connected());
    assert_eq!(waiting.proxy.connection_count(), 1);

    drop(waiting.session);
    waiting.proxy.shutdown().await.unwrap();
}

version_matrix!(a_silent_backend_fails_the_switch_and_the_player_stays; p763 = 763);
