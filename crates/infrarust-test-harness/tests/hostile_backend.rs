#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Mutex;
use std::time::Duration;

use infrarust_protocol::packets::config::CRegistryData;
use infrarust_test_harness::{
    DEFAULT_TIMEOUT, FakeBackend, ProtocolVersion, ScriptedPlugin, ServerSpec, TestProxy,
    holding_handler, version_matrix,
};

const T: Duration = DEFAULT_TIMEOUT;

const HOLD: &str = "hostile_backend_hold";
const REGISTRY_MARKER: &[u8] = b"hostile registry";

async fn a_backends_registry_never_reaches_the_next_limbo_player(version: ProtocolVersion) {
    let backend = FakeBackend::builder().hold_config().spawn().await.unwrap();
    let (handler, _holds) = holding_handler(HOLD);
    let handler = Mutex::new(Some(handler));
    let holder = ScriptedPlugin::new("holder").on_enable(move |ctx| {
        let handler = handler.lock().unwrap().take().unwrap();
        ctx.register_limbo_handler(handler).unwrap();
    });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .server(
            ServerSpec::offline("hub")
                .unreachable()
                .limbo_handlers([HOLD]),
        )
        .plugin(holder)
        .start()
        .await
        .unwrap();

    let first = proxy.client_for("lobby", version).unwrap();
    let login = tokio::spawn(async move { first.login("Alex").await });
    let mut conn = backend.next_connection(T).await.unwrap();
    conn.send_packet(&CRegistryData {
        data: REGISTRY_MARKER.to_vec(),
    })
    .await
    .unwrap();
    conn.finish_config(T).await.unwrap();
    let _first = tokio::time::timeout(T, login)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .joined()
        .unwrap();

    let second = proxy
        .client_for("hub", version)
        .unwrap()
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let frames = second.config_frames();
    assert!(!frames.is_empty(), "the limbo player got no registry data");
    assert!(
        frames
            .iter()
            .all(|frame| frame.payload.as_ref() != REGISTRY_MARKER),
        "a frame chosen by the backend reached another player"
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(a_backends_registry_never_reaches_the_next_limbo_player; p764 = 764, p766 = 766, p777 = 777);
