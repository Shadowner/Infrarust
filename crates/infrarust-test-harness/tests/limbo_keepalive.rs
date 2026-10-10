#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_test_harness::{
    DEFAULT_TIMEOUT, EventKind, FakeBackend, ProtocolVersion, Recorder, ServerSpec, TestProxy,
    holding_gate, next_hold,
};

const T: Duration = DEFAULT_TIMEOUT;
const KEEPALIVE_GRACE: Duration = Duration::from_secs(45);
const STEVE: &str = "Steve";
const GATE: &str = "gate";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_that_stops_answering_limbo_keepalives_does_not_count_as_quitting() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let (gate, mut holds) = holding_gate("gatekeeper", GATE);
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("lobby")
                .backend(backend.addr())
                .limbo_handlers([GATE]),
        )
        .plugin(gate)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let session = proxy
        .client(ProtocolVersion::V1_20_2)
        .ignore_keepalives()
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _hold = next_hold(&mut holds, T).await.unwrap();

    let disconnect = recorder
        .wait_for(
            |e| e.kind == EventKind::Disconnect && e.is_named(STEVE),
            KEEPALIVE_GRACE,
        )
        .await
        .unwrap();
    assert_eq!(disconnect.cause(), "error", "{disconnect:?}");

    let exit = recorder
        .wait_for_kind(EventKind::LimboExit, T)
        .await
        .unwrap();
    assert_eq!(exit.detail_str("reason"), "timed_out", "{exit:?}");
    session.quit().await;

    proxy.shutdown().await.unwrap();
}
