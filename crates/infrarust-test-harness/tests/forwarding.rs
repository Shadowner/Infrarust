#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_api::types::Component;
use infrarust_config::ProxyMode;
use infrarust_test_harness::{
    BackendConn, ClientSession, DEFAULT_TIMEOUT, EventKind, FakeBackend, HarnessError,
    ProtocolVersion, Recorded, Recorder, ServerSpec, TestProxy,
};
use serde_json::json;

const T: Duration = DEFAULT_TIMEOUT;
const ROUNDS: usize = 50;
const KICK_ROUNDS: usize = 20;
const SESSIONS_AT_SHUTDOWN: usize = 12;
const LONG_DRAIN: Duration = Duration::from_secs(60);

macro_rules! forwarded {
    ($body:ident) => {
        mod $body {
            use super::*;

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn passthrough_p47() {
                super::$body(ProxyMode::Passthrough, ProtocolVersion(47)).await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn passthrough_current() {
                super::$body(
                    ProxyMode::Passthrough,
                    ProtocolVersion(infrarust_test_harness::versions::CURRENT),
                )
                .await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn zero_copy_p47() {
                super::$body(ProxyMode::ZeroCopy, ProtocolVersion(47)).await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn zero_copy_current() {
                super::$body(
                    ProxyMode::ZeroCopy,
                    ProtocolVersion(infrarust_test_harness::versions::CURRENT),
                )
                .await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn server_only_p47() {
                super::$body(ProxyMode::ServerOnly, ProtocolVersion(47)).await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn server_only_current() {
                super::$body(
                    ProxyMode::ServerOnly,
                    ProtocolVersion(infrarust_test_harness::versions::CURRENT),
                )
                .await;
            }
        }
    };
}

struct Pipe {
    proxy: TestProxy,
    backend: FakeBackend,
    recorder: Recorder,
}

async fn pipe(mode: ProxyMode, drain: Option<Duration>) -> Pipe {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let mut builder = TestProxy::builder()
        .server(ServerSpec::new("lobby", mode).backend(backend.addr()))
        .plugin(recorder.plugin());
    if let Some(drain) = drain {
        builder = builder.drain_timeout(drain);
    }
    let proxy = builder.start().await.unwrap();
    Pipe {
        proxy,
        backend,
        recorder,
    }
}

impl Pipe {
    async fn join(&self, version: ProtocolVersion, username: &str) -> (ClientSession, BackendConn) {
        let session = self
            .proxy
            .client(version)
            .login(username)
            .await
            .unwrap()
            .joined()
            .unwrap();
        let conn = self.backend.next_connection(T).await.unwrap();
        assert_eq!(conn.username(), username);
        self.proxy.wait_for_player(username, T).await.unwrap();
        (session, conn)
    }

    async fn disconnect_of(&self, username: &str) -> Recorded {
        self.recorder
            .wait_for(
                |e| e.kind == EventKind::Disconnect && e.username.as_deref() == Some(username),
                T,
            )
            .await
            .unwrap()
    }
}

async fn closed_by_proxy(session: &mut ClientSession) {
    loop {
        match session.recv_frame(T).await {
            Ok(_) => {}
            Err(HarnessError::Closed(_)) => return,
            Err(other) => panic!("expected the proxy to close the client, got {other:?}"),
        }
    }
}

async fn assert_proxy_let_go(conn: &mut BackendConn) {
    tokio::time::timeout(T, async {
        while conn
            .send_system_message_json(r#"{"text":"anyone there?"}"#)
            .await
            .is_ok()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the proxy kept its backend socket open");
}

fn assert_cause(disconnect: &Recorded, cause: &str, context: &str) {
    assert_eq!(
        disconnect.detail["cause"],
        json!(cause),
        "{context}: {disconnect:?}"
    );
}

async fn a_client_quit_is_reported_as_client_quit(mode: ProxyMode, version: ProtocolVersion) {
    let pipe = pipe(mode, None).await;

    for round in 0..ROUNDS {
        let username = format!("Quitter{round}");
        let (session, mut conn) = pipe.join(version, &username).await;

        session.quit().await;
        conn.closed(T).await.unwrap();
        conn.close().await;

        let disconnect = pipe.disconnect_of(&username).await;
        assert_cause(&disconnect, "client_quit", &format!("round {round}"));
        assert_eq!(disconnect.detail["reason"], json!(null));
    }

    pipe.proxy.wait_for_connection_count(0, T).await.unwrap();
    pipe.proxy.shutdown().await.unwrap();
}

forwarded!(a_client_quit_is_reported_as_client_quit);

async fn a_backend_close_is_reported_as_backend_closed(mode: ProxyMode, version: ProtocolVersion) {
    let pipe = pipe(mode, None).await;

    for round in 0..ROUNDS {
        let username = format!("Stayer{round}");
        let (mut session, mut conn) = pipe.join(version, &username).await;

        conn.close().await;
        closed_by_proxy(&mut session).await;
        conn.closed(T).await.unwrap();

        let disconnect = pipe.disconnect_of(&username).await;
        assert_cause(&disconnect, "backend_closed", &format!("round {round}"));
        assert_eq!(disconnect.detail["reason"], json!(null));
    }

    pipe.proxy.wait_for_connection_count(0, T).await.unwrap();
    pipe.proxy.shutdown().await.unwrap();
}

forwarded!(a_backend_close_is_reported_as_backend_closed);

async fn shutdown_does_not_wait_for_a_half_closed_session(
    mode: ProxyMode,
    version: ProtocolVersion,
) {
    let pipe = pipe(mode, Some(LONG_DRAIN)).await;
    let (mut session, mut conn) = pipe.join(version, "Steve").await;

    session.close_write().await.unwrap();
    conn.closed(T).await.unwrap();
    conn.send_system_message_json(r#"{"text":"still here"}"#)
        .await
        .unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "still here");

    let Pipe {
        proxy, recorder, ..
    } = pipe;
    tokio::time::timeout(T, proxy.shutdown())
        .await
        .expect("a half-closed session must not hold shutdown until the drain timeout")
        .unwrap();

    closed_by_proxy(&mut session).await;
    assert_proxy_let_go(&mut conn).await;
    let disconnects = recorder.of(EventKind::Disconnect);
    assert_eq!(disconnects.len(), 1, "{disconnects:?}");
    assert_cause(&disconnects[0], "client_quit", "half-closed by the client");
}

forwarded!(shutdown_does_not_wait_for_a_half_closed_session);

async fn a_shutdown_is_reported_as_shutdown(mode: ProxyMode, version: ProtocolVersion) {
    let pipe = pipe(mode, Some(LONG_DRAIN)).await;
    let mut open = Vec::new();
    for index in 0..SESSIONS_AT_SHUTDOWN {
        open.push(pipe.join(version, &format!("Sleeper{index}")).await);
    }

    let Pipe {
        proxy, recorder, ..
    } = pipe;
    tokio::time::timeout(T, proxy.shutdown())
        .await
        .expect("open sessions must close on shutdown")
        .unwrap();

    for (session, conn) in &mut open {
        closed_by_proxy(session).await;
        conn.closed(T).await.unwrap();
    }
    let disconnects = recorder.of(EventKind::Disconnect);
    assert_eq!(disconnects.len(), SESSIONS_AT_SHUTDOWN, "{disconnects:?}");
    for disconnect in &disconnects {
        assert_cause(disconnect, "shutdown", "proxy shutdown");
    }
}

forwarded!(a_shutdown_is_reported_as_shutdown);

async fn a_kick_is_reported_as_kicked(mode: ProxyMode, version: ProtocolVersion) {
    let pipe = pipe(mode, None).await;

    for round in 0..KICK_ROUNDS {
        let username = format!("Kicked{round}");
        let (mut session, conn) = pipe.join(version, &username).await;
        let player = pipe.proxy.wait_for_player(&username, T).await.unwrap();

        player.disconnect(Component::text("Bye")).await;
        closed_by_proxy(&mut session).await;
        conn.closed(T).await.unwrap();

        let disconnect = pipe.disconnect_of(&username).await;
        assert_cause(&disconnect, "kicked", &format!("round {round}"));
        assert_eq!(disconnect.detail["reason"], json!("Bye"));
    }

    pipe.proxy.wait_for_connection_count(0, T).await.unwrap();
    pipe.proxy.shutdown().await.unwrap();
}

forwarded!(a_kick_is_reported_as_kicked);
