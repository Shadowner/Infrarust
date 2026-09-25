#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use infrarust_api::event::bus::{EventBusExt, FireError};
use infrarust_api::event::{Event, EventPriority};
use infrarust_api::events::lifecycle::{PostLoginEvent, PreLoginEvent};
use infrarust_test_harness::versions::CURRENT;
use infrarust_test_harness::{
    DEFAULT_TIMEOUT, EventKind, FakeBackend, ProtocolVersion, Recorder, ScriptedPlugin, ServerSpec,
    TestProxy,
};
use tokio::sync::mpsc;

const T: Duration = DEFAULT_TIMEOUT;

struct Greeting {
    to: String,
    reply: Option<String>,
}
impl Event for Greeting {}

type Exchange = (Result<Option<String>, FireError>, Result<(), FireError>);

fn forged_pre_login(event: &PostLoginEvent) -> PreLoginEvent {
    let mut profile = event.profile.clone();
    profile.username = "Forged".to_string();
    PreLoginEvent::new(
        profile,
        "127.0.0.1:1".parse().unwrap(),
        event.protocol_version,
        "lobby.test".to_string(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugins_exchange_a_custom_event_through_a_running_proxy() {
    let (replies, mut replied) = mpsc::unbounded_channel::<Exchange>();
    let asker = ScriptedPlugin::new("asker").on_enable(move |ctx| {
        let bus = ctx.event_bus_handle();
        let replies = replies.clone();
        ctx.event_bus()
            .subscribe_async::<PostLoginEvent, _>(EventPriority::NORMAL, move |event| {
                let bus = Arc::clone(&bus);
                let replies = replies.clone();
                let to = event.profile.username.clone();
                let forged = forged_pre_login(event);
                Box::pin(async move {
                    let answered = bus.fire(Greeting { to, reply: None }).await;
                    let forged = bus.fire(forged).await.map(drop);
                    let _ = replies.send((answered.map(|greeting| greeting.reply), forged));
                })
            });
    });
    let answerer =
        ScriptedPlugin::new("answerer").on::<Greeting>(EventPriority::NORMAL, |greeting| {
            greeting.reply = Some(format!("welcome, {}", greeting.to));
        });
    let recorder = Recorder::new();
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(asker)
        .plugin(answerer)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let session = proxy
        .client(ProtocolVersion(CURRENT))
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();
    let (reply, forged) = tokio::time::timeout(T, replied.recv())
        .await
        .expect("the asker never heard back")
        .unwrap();

    assert_eq!(reply, Ok(Some("welcome, Steve".to_string())));
    assert_eq!(forged, Err(FireError::Reserved));
    assert!(recorder.for_username("Forged").is_empty());
    assert_eq!(recorder.count(EventKind::PreLogin), 1);

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adding_a_server_file_delivers_a_config_reload() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    tokio::time::timeout(T, proxy.bus().flush())
        .await
        .expect("the event queue never drained");
    assert_eq!(recorder.count(EventKind::ConfigReload), 0);

    std::fs::write(
        proxy.dir().join("servers").join("extra.toml"),
        format!(
            "domains = [\"extra.test\"]\naddresses = [\"{}\"]\n",
            backend.addr()
        ),
    )
    .unwrap();

    recorder
        .wait_for_kind(EventKind::ConfigReload, T)
        .await
        .unwrap();

    proxy.shutdown().await.unwrap();
}
