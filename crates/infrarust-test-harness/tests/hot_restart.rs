#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::future::Future;
use std::ops::Range;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use infrarust_api::event::{EventPriority, ResultedEvent};
use infrarust_api::events::lifecycle::{LoginEvent, PermissionsSetupEvent, PermissionsSetupResult};
use infrarust_api::permissions::PermissionMap;
use infrarust_api::services::proxy_control::RestartRequestError;
use infrarust_core::hot_restart::{InheritanceOutcome, RESUME_COOKIE, RestartPhase};
use infrarust_core::test_support::FakePreflight;
use infrarust_protocol::packets::cookie::CStoreCookie;
use infrarust_protocol::packets::play::chat::SChatMessage;
use infrarust_protocol::packets::play::transfer::CTransfer;
use infrarust_server_manager::{ProviderStatus, ServerManagerError, ServerProvider};
use infrarust_test_harness::{
    BackendConn, ClientSession, DEFAULT_TIMEOUT, EventKind, FakeBackend, FakeClient, HarnessError,
    ProtocolVersion, ProxyEnd, Recorder, ScriptedPlugin, ServerSpec, TestProxy,
};
use tokio::sync::Notify;

const T: Duration = DEFAULT_TIMEOUT;
const RESTART: Duration = Duration::from_secs(20);
const VERSION: ProtocolVersion = ProtocolVersion(777);
const GRANTED: &str = "hot.restart.granted";

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let waited = tokio::time::timeout(T, async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "timed out waiting for {what}");
}

fn held_login(username: &'static str) -> (ScriptedPlugin, Arc<Notify>, Arc<Notify>) {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let gate = {
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        ScriptedPlugin::new("gate").on_async::<LoginEvent>(EventPriority::NORMAL, move |event| {
            let hold = event.profile().username == username;
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            Box::pin(async move {
                if hold {
                    entered.notify_one();
                    release.notified().await;
                }
            })
        })
    };
    (gate, entered, release)
}

struct Pipe {
    name: &'static str,
    client: ClientSession,
    backend: BackendConn,
}

impl Pipe {
    async fn join(
        proxy: &TestProxy,
        server: &str,
        backend: &FakeBackend,
        name: &'static str,
    ) -> Self {
        let client = proxy
            .client_for(server, VERSION)
            .unwrap()
            .login(name)
            .await
            .unwrap()
            .joined()
            .unwrap();
        let backend = backend.next_connection(T).await.unwrap();
        proxy.wait_for_player(name, T).await.unwrap();
        Self {
            name,
            client,
            backend,
        }
    }

    async fn send(&mut self, numbers: Range<u32>) {
        for n in numbers {
            self.client
                .chat(&format!("{}-up-{n}", self.name))
                .await
                .unwrap();
            self.backend
                .send_system_message_json(&format!(r#"{{"text":"{}-down-{n}"}}"#, self.name))
                .await
                .unwrap();
        }
    }

    async fn receive(&mut self, numbers: Range<u32>) {
        for n in numbers.clone() {
            let up = self.backend.expect::<SChatMessage>(T).await.unwrap();
            assert_eq!(up.message, format!("{}-up-{n}", self.name));
        }
        for n in numbers {
            let down = self.client.expect_system_text(T).await.unwrap();
            assert_eq!(down, format!("{}-down-{n}", self.name));
        }
    }

    async fn exchange(&mut self, numbers: Range<u32>) {
        self.send(numbers.clone()).await;
        self.receive(numbers).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forwarding_sessions_survive_a_hot_restart_and_intercepted_ones_reconnect() {
    let pipe = FakeBackend::builder().spawn().await.unwrap();
    let splice = FakeBackend::builder().spawn().await.unwrap();
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let (gate, entered, release) = held_login("Late");
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("pipe").backend(pipe.addr()))
        .server(ServerSpec::zero_copy("splice").backend(splice.addr()))
        .server(ServerSpec::offline("lobby").backend(lobby.addr()))
        .plugin(gate)
        .hot_restart()
        .start()
        .await
        .unwrap();
    let addr = proxy.addr();
    let mut steve = Pipe::join(&proxy, "pipe", &pipe, "Steve").await;
    let mut alex = Pipe::join(&proxy, "splice", &splice, "Alex").await;
    let mut notch = proxy
        .client_for("lobby", VERSION)
        .unwrap()
        .login("Notch")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _notch_backend = lobby.next_connection(T).await.unwrap();
    let steve_id = proxy.wait_for_player("Steve", T).await.unwrap().id();
    proxy.wait_for_player("Notch", T).await.unwrap();
    steve.exchange(0..10).await;
    alex.exchange(0..10).await;
    let late_client = proxy.client_for("pipe", VERSION).unwrap();
    let late = tokio::spawn(async move { late_client.login("Late").await });
    tokio::time::timeout(T, entered.notified())
        .await
        .expect("Late must reach the Login event");
    steve.send(10..40).await;
    alex.send(10..40).await;

    let run = proxy.run_to_exit().unwrap();
    let hub = Arc::clone(run.hub().expect("the proxy runs with hot restart"));
    hub.request().unwrap();
    assert_eq!(hub.request(), Err(RestartRequestError::InProgress));
    steve.send(40..60).await;
    alex.send(40..60).await;
    wait_until("the grace", || hub.phase() == RestartPhase::Draining).await;
    assert_eq!(hub.request(), Err(RestartRequestError::InProgress));
    release.notify_one();
    let cookie = notch.expect::<CStoreCookie>(T).await.unwrap();
    assert_eq!(cookie.key, RESUME_COOKIE);
    let sent_back = notch.expect::<CTransfer>(T).await.unwrap();
    assert_eq!(sent_back.port, i32::from(addr.port()));
    let notch_cookies = notch.cookies();
    notch.quit().await;

    let ProxyEnd::Restarted(successor) = run.finish(RESTART).await.unwrap() else {
        panic!("the restart must hand over");
    };
    assert_eq!(successor.handover().package.state.sessions.len(), 3);
    assert_eq!(successor.handover().package.state.resumes.len(), 1);
    let after = Recorder::new();
    let grants =
        ScriptedPlugin::new("grants").on::<PermissionsSetupEvent>(EventPriority::NORMAL, |event| {
            if event.profile().username == "Steve" {
                event.set_result(PermissionsSetupResult::Custom(Arc::new(
                    PermissionMap::new().with(GRANTED, true),
                )));
            }
        });
    let proxy = successor
        .builder()
        .plugin(after.plugin())
        .plugin(grants)
        .start()
        .await
        .unwrap();

    assert_eq!(proxy.addr(), addr);
    assert_eq!(
        proxy.inheritance().unwrap().outcome(),
        InheritanceOutcome::Committed
    );
    let late_client = tokio::time::timeout(T, late)
        .await
        .expect("Late's login must end")
        .unwrap()
        .unwrap()
        .joined()
        .expect("a login in flight during the grace completes, even across the handover");
    let late_backend = pipe.next_connection(T).await.unwrap();
    let mut late = Pipe {
        name: "Late",
        client: late_client,
        backend: late_backend,
    };
    steve.receive(10..60).await;
    alex.receive(10..60).await;
    steve.exchange(60..80).await;
    alex.exchange(60..80).await;
    late.exchange(0..20).await;
    assert_eq!(
        proxy.wait_for_player("Steve", T).await.unwrap().id(),
        steve_id
    );
    assert_eq!(proxy.connection_count(), 3);
    for name in ["Steve", "Alex", "Late"] {
        let kinds: Vec<EventKind> = after.for_username(name).iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [EventKind::PermissionsSetup], "{name}");
    }
    assert!(
        proxy
            .wait_for_player("Steve", T)
            .await
            .unwrap()
            .has_permission(GRANTED)
    );
    assert!(
        !proxy
            .wait_for_player("Alex", T)
            .await
            .unwrap()
            .has_permission(GRANTED)
    );

    let mut fresh = Pipe::join(&proxy, "pipe", &pipe, "Fresh").await;
    fresh.exchange(0..5).await;
    assert_eq!(after.count(EventKind::PostLogin), 1);
    let _notch = proxy
        .client_for("lobby", VERSION)
        .unwrap()
        .transferred()
        .cookies(notch_cookies)
        .login("Notch")
        .await
        .unwrap()
        .joined()
        .expect("the player sent back before the restart comes back on its own");
    assert_eq!(lobby.next_connection(T).await.unwrap().username(), "Notch");
    assert_eq!(after.count(EventKind::PostLogin), 2);
    let hub = proxy
        .restart_hub()
        .expect("the successor can restart again");
    assert_eq!(hub.phase(), RestartPhase::Idle);
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stop_during_the_grace_is_a_normal_stop() {
    let pipe = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let (gate, entered, release) = held_login("Late");
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("pipe").backend(pipe.addr()))
        .plugin(gate)
        .plugin(recorder.plugin())
        .hot_restart()
        .start()
        .await
        .unwrap();
    let addr = proxy.addr();
    let steve = Pipe::join(&proxy, "pipe", &pipe, "Steve").await;
    let late_client = proxy.client_for("pipe", VERSION).unwrap();
    let late = tokio::spawn(async move { late_client.login("Late").await });
    tokio::time::timeout(T, entered.notified())
        .await
        .expect("Late must reach the Login event");

    let run = proxy.run_to_exit().unwrap();
    let server = Arc::clone(run.server());
    let hub = Arc::clone(run.hub().unwrap());
    hub.request().unwrap();
    wait_until("the grace", || hub.phase() == RestartPhase::Draining).await;
    run.shutdown_token().cancel();
    release.notify_one();

    assert!(matches!(
        run.finish(RESTART).await.unwrap(),
        ProxyEnd::Stopped
    ));
    steve.backend.closed(T).await.unwrap();
    if let Ok(mut late) = tokio::time::timeout(T, late)
        .await
        .expect("Late's login must end")
        .unwrap()
        .and_then(|outcome| outcome.joined())
    {
        loop {
            match late.recv_frame(T).await {
                Ok(_) => {}
                Err(HarnessError::Timeout { .. }) => {
                    panic!("a session that joined during the stop must be closed")
                }
                Err(_) => break,
            }
        }
    }
    assert_eq!(server.services().connection_registry.count(), 0);
    assert_eq!(server.relays().active(), 0);
    assert_eq!(hub.request(), Err(RestartRequestError::ShuttingDown));
    let disconnect = recorder
        .wait_for(
            |e| e.kind == EventKind::Disconnect && e.is_named("Steve"),
            T,
        )
        .await
        .unwrap();
    assert_eq!(disconnect.cause(), "shutdown");
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_preflight_keeps_the_sessions_and_the_listener() {
    let pipe = FakeBackend::builder().spawn().await.unwrap();
    let preflight = FakePreflight::refusing();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("pipe").backend(pipe.addr()))
        .hot_restart_with(preflight.clone())
        .start()
        .await
        .unwrap();
    let domain = proxy.domain("pipe").unwrap().to_string();
    let mut steve = Pipe::join(&proxy, "pipe", &pipe, "Steve").await;

    let run = proxy.run_to_exit().unwrap();
    let hub = Arc::clone(run.hub().unwrap());
    hub.request().unwrap();
    wait_until("the preflight to fail", || hub.last_failure().is_some()).await;

    assert_eq!(hub.phase(), RestartPhase::Idle);
    assert_eq!(preflight.checks(), 1);
    steve.exchange(0..10).await;
    let fresh = FakeClient::new(run.addr(), VERSION)
        .domain(domain)
        .login("Fresh")
        .await
        .unwrap()
        .joined()
        .unwrap();
    pipe.next_connection(T).await.unwrap();
    drop(fresh);
    assert!(!run.is_finished());

    run.shutdown_token().cancel();
    assert!(matches!(
        run.finish(RESTART).await.unwrap(),
        ProxyEnd::Stopped
    ));
}

struct ChildServer;

type ProviderFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ServerManagerError>> + Send + 'a>>;

impl ServerProvider for ChildServer {
    fn start(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn stop(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn check_status(&self) -> ProviderFuture<'_, ProviderStatus> {
        Box::pin(async { Ok(ProviderStatus::Running) })
    }

    fn provider_type(&self) -> &'static str {
        "child"
    }

    fn owns_child_process(&self) -> bool {
        true
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_running_child_server_refuses_the_restart_before_any_probe() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let preflight = FakePreflight::approving();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::passthrough("survival")
                .backend(backend.addr())
                .managed(Arc::new(ChildServer)),
        )
        .hot_restart_with(preflight.clone())
        .start()
        .await
        .unwrap();
    let manager = Arc::clone(proxy.services().server_manager.as_ref().unwrap());
    wait_until("the child server to run", || {
        !manager.running_child_processes().is_empty()
    })
    .await;

    let run = proxy.run_to_exit().unwrap();
    let hub = Arc::clone(run.hub().unwrap());
    hub.request().unwrap();
    wait_until("the restart to be refused", || hub.last_failure().is_some()).await;

    let failure = hub.last_failure().unwrap();
    assert!(failure.contains("survival"), "{failure}");
    assert!(failure.contains("orphaned"), "{failure}");
    assert_eq!(preflight.checks(), 0);
    assert_eq!(hub.phase(), RestartPhase::Idle);

    run.shutdown_token().cancel();
    assert!(matches!(
        run.finish(RESTART).await.unwrap(),
        ProxyEnd::Stopped
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_successor_that_cannot_bind_its_new_address_keeps_the_inherited_listener() {
    let pipe = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("pipe").backend(pipe.addr()))
        .hot_restart()
        .start()
        .await
        .unwrap();
    let addr = proxy.addr();
    let mut steve = Pipe::join(&proxy, "pipe", &pipe, "Steve").await;
    steve.exchange(0..10).await;

    let run = proxy.run_to_exit().unwrap();
    run.hub()
        .expect("the proxy runs with hot restart")
        .request()
        .unwrap();
    let ProxyEnd::Restarted(successor) = run.finish(RESTART).await.unwrap() else {
        panic!("the restart must hand over");
    };

    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let config_path = successor.config_path();
    let mut config: toml::Table = std::fs::read_to_string(&config_path)
        .unwrap()
        .parse()
        .unwrap();
    config.insert(
        "bind".into(),
        toml::Value::String(taken.local_addr().unwrap().to_string()),
    );
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();

    let proxy = successor.builder().start().await.unwrap();
    assert_eq!(proxy.addr(), addr);
    steve.exchange(10..30).await;
    let mut fresh = Pipe::join(&proxy, "pipe", &pipe, "Fresh").await;
    fresh.exchange(0..5).await;
    proxy.shutdown().await.unwrap();
}
