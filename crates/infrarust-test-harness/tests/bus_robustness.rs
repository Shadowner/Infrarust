#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use infrarust_api::event::EventPriority;
use infrarust_api::events::lifecycle::PreLoginEvent;
use infrarust_api::events::proxy::ServerStateChangeEvent;
use infrarust_core::event_bus::{DiagnosticKind, EventBusImpl, HandlerDiagnostic};
use infrarust_plugin_server_wake::ServerWakePlugin;
use infrarust_test_harness::versions::CURRENT;
use infrarust_test_harness::{
    DEFAULT_TIMEOUT, FakeBackend, ProtocolVersion, ScriptedPlugin, ServerSpec, TestProxy,
};
use tokio::sync::broadcast::Receiver;
use toml::{Table, Value};

const T: Duration = DEFAULT_TIMEOUT;

async fn diagnostic_from(
    diagnostics: &mut Receiver<HandlerDiagnostic>,
    owner: &str,
) -> HandlerDiagnostic {
    tokio::time::timeout(T, async {
        loop {
            let diagnostic = diagnostics.recv().await.unwrap();
            if &*diagnostic.owner == owner {
                return diagnostic;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no diagnostic from {owner}"))
}

fn owners_of_state_changes(bus: &EventBusImpl) -> Vec<String> {
    bus.listener_owners::<ServerStateChangeEvent>()
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panicking_prelogin_handler_does_not_stop_the_login() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let plugin = ScriptedPlugin::new("panicky")
        .on::<PreLoginEvent>(EventPriority::NORMAL, |_| panic!("prelogin boom"));
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .start()
        .await
        .unwrap();
    let mut diagnostics = proxy.bus().diagnostics();

    let session = proxy
        .client(ProtocolVersion(CURRENT))
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let conn = backend.next_connection(T).await.unwrap();

    assert_eq!(conn.username(), "Steve");
    let diagnostic = diagnostic_from(&mut diagnostics, "panicky").await;
    assert_eq!(diagnostic.event, "PreLoginEvent");
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Panicked {
            message: "prelogin boom".to_string()
        }
    );

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hung_prelogin_handler_delays_the_login_by_the_timeout_only() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let plugin = ScriptedPlugin::new("stuck")
        .on_async::<PreLoginEvent>(EventPriority::NORMAL, |_| Box::pin(std::future::pending()));
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .patch_config(|table| {
            table.insert(
                "events".into(),
                Value::Table(Table::from_iter([(
                    "handler_timeout".into(),
                    Value::String("200ms".into()),
                )])),
            );
        })
        .start()
        .await
        .unwrap();
    let mut diagnostics = proxy.bus().diagnostics();

    let started = Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        proxy.client(ProtocolVersion(CURRENT)).login("Alex"),
    )
    .await
    .expect("the login never finished");
    let elapsed = started.elapsed();

    let session = outcome.unwrap().joined().unwrap();
    assert!(
        elapsed >= Duration::from_millis(200),
        "joined after {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(5), "joined after {elapsed:?}");
    assert_eq!(backend.next_connection(T).await.unwrap().username(), "Alex");
    let diagnostic = diagnostic_from(&mut diagnostics, "stuck").await;
    assert_eq!(diagnostic.event, "PreLoginEvent");
    assert_eq!(diagnostic.kind, DiagnosticKind::TimedOut);

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_wake_follows_state_changes_through_its_own_bus_listener() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(ServerWakePlugin::new())
        .start()
        .await
        .unwrap();
    let bus = Arc::clone(proxy.bus());

    assert_eq!(owners_of_state_changes(&bus), ["server_wake"]);

    proxy.shutdown().await.unwrap();
    assert!(owners_of_state_changes(&bus).is_empty());
}
