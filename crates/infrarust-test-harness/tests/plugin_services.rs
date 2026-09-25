#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use infrarust_api::error::ServiceError;
use infrarust_api::services::{ServiceRegistry, ServiceRegistryExt};
use infrarust_test_harness::{
    EventKind, RECORDER_PLUGIN_ID, Recorded, Recorder, ScriptedPlugin, TestProxy,
};
use serde_json::{Value, json};

trait LoginState: Send + Sync {
    fn is_logged_in(&self, username: &str) -> bool;
}

struct OnlySteve;

impl LoginState for OnlySteve {
    fn is_logged_in(&self, username: &str) -> bool {
        username == "Steve"
    }
}

#[derive(Default)]
struct Seen {
    found: Option<bool>,
    refused: Option<ServiceError>,
    view: Option<Arc<dyn ServiceRegistry>>,
}

fn plugins_of(recorder: &Recorder, kind: EventKind) -> Vec<Value> {
    recorder
        .of(kind)
        .iter()
        .map(|e| e.detail["plugin"].clone())
        .collect()
}

fn position(events: &[Recorded], pick: impl Fn(&Recorded) -> bool) -> u64 {
    events
        .iter()
        .find(|e| pick(e))
        .map(|e| e.seq)
        .unwrap_or_else(|| panic!("event not recorded in {events:#?}"))
}

#[tokio::test]
async fn a_service_is_shared_refused_twice_and_withdrawn_with_its_provider() {
    let recorder = Recorder::new();
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let provider = ScriptedPlugin::new("auth")
        .after(RECORDER_PLUGIN_ID)
        .on_enable(|ctx| {
            ctx.services()
                .provide::<dyn LoginState>(Arc::new(OnlySteve))
                .expect("the first provider is accepted");
        });
    let slot = Arc::clone(&seen);
    let consumer = ScriptedPlugin::new("lobby")
        .after("auth")
        .on_enable(move |ctx| {
            let mut seen = slot.lock().unwrap();
            seen.found = ctx
                .services()
                .get::<dyn LoginState>()
                .map(|state| state.is_logged_in("Steve"));
            seen.refused = ctx
                .services()
                .provide::<dyn LoginState>(Arc::new(OnlySteve))
                .err();
            seen.view = Some(ctx.services_handle());
        });
    let proxy = TestProxy::builder()
        .plugin(recorder.plugin())
        .plugin(provider)
        .plugin(consumer)
        .start()
        .await
        .unwrap();

    let view = {
        let mut seen = seen.lock().unwrap();
        assert_eq!(seen.found, Some(true));
        assert!(
            matches!(&seen.refused, Some(ServiceError::AlreadyProvided { by, .. }) if by == "auth"),
            "{:?}",
            seen.refused
        );
        seen.view.take().unwrap()
    };
    assert_eq!(view.provider::<dyn LoginState>().as_deref(), Some("auth"));
    let provided = recorder.of(EventKind::ServiceProvided);
    assert_eq!(provided.len(), 1, "{provided:#?}");
    assert_eq!(provided[0].detail["provider"], json!("auth"));
    assert!(
        provided[0].detail["service"]
            .as_str()
            .unwrap()
            .contains("LoginState")
    );

    proxy.disable_plugin("auth").await.unwrap();

    assert!(view.get::<dyn LoginState>().is_none());
    let removed = recorder.of(EventKind::ServiceRemoved);
    assert_eq!(removed.len(), 1, "{removed:#?}");
    assert_eq!(removed[0].detail["provider"], json!("auth"));
    let events = recorder.events();
    assert!(
        position(&events, |e| e.kind == EventKind::ServiceRemoved)
            < position(&events, |e| e.kind == EventKind::PluginDisabled),
        "{events:#?}"
    );

    let handle = view
        .provide::<dyn LoginState>(Arc::new(OnlySteve))
        .expect("the slot is free again");
    assert_eq!(view.provider::<dyn LoginState>().as_deref(), Some("lobby"));
    assert!(handle.withdraw());
    assert!(view.get::<dyn LoginState>().is_none());
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_plugin_disable_is_refused_while_another_plugin_requires_it() {
    let proxy = TestProxy::builder()
        .plugin(ScriptedPlugin::new("core_lib"))
        .plugin(ScriptedPlugin::new("addon").depends_on("core_lib"))
        .start()
        .await
        .unwrap();

    assert!(proxy.disable_plugin("core_lib").await.is_err());
    proxy.disable_plugin("addon").await.unwrap();
    proxy.disable_plugin("core_lib").await.unwrap();
    assert!(proxy.disable_plugin("core_lib").await.is_err());
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn plugin_lifecycle_events_follow_the_enable_and_disable_order() {
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .plugin(ScriptedPlugin::new("beta").after("alpha"))
        .plugin(recorder.plugin())
        .plugin(ScriptedPlugin::new("alpha").after(RECORDER_PLUGIN_ID))
        .start()
        .await
        .unwrap();

    assert_eq!(
        plugins_of(&recorder, EventKind::PluginEnabled),
        [json!(RECORDER_PLUGIN_ID), json!("alpha"), json!("beta")]
    );
    assert!(
        recorder
            .of(EventKind::PluginEnabled)
            .iter()
            .all(|e| e.detail["version"] == json!("0.0.0"))
    );
    let events = recorder.events();
    assert!(
        position(&events, |e| e.kind == EventKind::PluginEnabled
            && e.detail["plugin"] == json!("beta"))
            < position(&events, |e| e.kind == EventKind::ProxyInitialize),
        "{events:#?}"
    );
    assert!(recorder.of(EventKind::PluginDisabled).is_empty());

    proxy.shutdown().await.unwrap();

    assert_eq!(
        plugins_of(&recorder, EventKind::PluginDisabled),
        [json!("beta"), json!("alpha")],
        "the recorder is disabled last and stops listening before its own event"
    );
    let events = recorder.events();
    assert!(
        position(&events, |e| e.kind == EventKind::ProxyShutdown)
            < position(&events, |e| e.kind == EventKind::PluginDisabled),
        "{events:#?}"
    );
}
