#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use infrarust_api::event::EventPriority;
use infrarust_api::event::bus::EventBusExt;
use infrarust_api::events::plugin::{PluginDisabledEvent, PluginEnabledEvent};
use infrarust_api::services::plugin_registry::PluginRegistry;
use infrarust_test_harness::{ScriptedPlugin, TestProxy};

type Sightings = Arc<Mutex<Vec<(String, Option<String>)>>>;
type View = Arc<Mutex<Option<(Option<String>, Vec<String>)>>>;

fn state_of(registry: &dyn PluginRegistry, id: &str) -> Option<String> {
    registry.plugin_info(id).map(|info| info.state)
}

fn ids(registry: &dyn PluginRegistry) -> Vec<String> {
    registry
        .list_plugin_info()
        .into_iter()
        .map(|info| info.id)
        .collect()
}

fn sighting(id: &str, state: Option<&str>) -> (String, Option<String>) {
    (id.to_owned(), state.map(str::to_owned))
}

#[tokio::test]
async fn a_plugin_enabled_before_is_in_the_registry_during_on_enable() {
    let seen: View = Arc::default();
    let slot = Arc::clone(&seen);
    let second = ScriptedPlugin::new("second")
        .depends_on("first")
        .on_enable(move |ctx| {
            let registry = ctx.plugin_registry();
            *slot.lock().unwrap() = Some((state_of(registry, "first"), ids(registry)));
        });

    let proxy = TestProxy::builder()
        .plugin(ScriptedPlugin::new("first"))
        .plugin(second)
        .start()
        .await
        .unwrap();

    assert_eq!(
        *seen.lock().unwrap(),
        Some((Some("enabled".to_owned()), vec!["first".to_owned()])),
        "the second plugin sees the first as enabled, and not itself yet"
    );
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_plugin_enabled_listener_finds_the_plugin_it_names() {
    let seen: Sightings = Arc::default();
    let slot = Arc::clone(&seen);
    let watcher = ScriptedPlugin::new("watcher").on_enable(move |ctx| {
        let registry = ctx.plugin_registry_handle();
        let slot = Arc::clone(&slot);
        ctx.event_bus().subscribe(
            EventPriority::NORMAL,
            move |event: &mut PluginEnabledEvent| {
                let state = state_of(registry.as_ref(), &event.plugin_id);
                slot.lock().unwrap().push((event.plugin_id.clone(), state));
            },
        );
    });

    let proxy = TestProxy::builder()
        .plugin(watcher)
        .plugin(ScriptedPlugin::new("late").after("watcher"))
        .start()
        .await
        .unwrap();

    assert_eq!(
        *seen.lock().unwrap(),
        [
            sighting("watcher", Some("enabled")),
            sighting("late", Some("enabled")),
        ]
    );
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_disabled_plugin_leaves_the_registry_before_its_event() {
    let seen: Sightings = Arc::default();
    let slot = Arc::clone(&seen);
    let watcher = ScriptedPlugin::new("watcher").on_enable(move |ctx| {
        let registry = ctx.plugin_registry_handle();
        let slot = Arc::clone(&slot);
        ctx.event_bus().subscribe(
            EventPriority::NORMAL,
            move |event: &mut PluginDisabledEvent| {
                let state = state_of(registry.as_ref(), &event.plugin_id);
                slot.lock().unwrap().push((event.plugin_id.clone(), state));
            },
        );
    });
    let proxy = TestProxy::builder()
        .plugin(watcher)
        .plugin(ScriptedPlugin::new("leaving"))
        .start()
        .await
        .unwrap();
    let registry = proxy
        .plugin_context("watcher")
        .await
        .unwrap()
        .plugin_registry_handle();
    assert_eq!(
        state_of(registry.as_ref(), "leaving").as_deref(),
        Some("enabled")
    );

    proxy.disable_plugin("leaving").await.unwrap();

    assert_eq!(state_of(registry.as_ref(), "leaving"), None);
    assert_eq!(ids(registry.as_ref()), ["watcher"]);
    assert_eq!(*seen.lock().unwrap(), [sighting("leaving", None)]);
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn plugins_disabled_on_shutdown_leave_the_registry_one_by_one() {
    let registry_slot: Arc<Mutex<Option<Arc<dyn PluginRegistry>>>> = Arc::default();
    let seen_on_disable: Arc<Mutex<Option<Vec<String>>>> = Arc::default();
    let enable_slot = Arc::clone(&registry_slot);
    let disable_slot = Arc::clone(&registry_slot);
    let seen = Arc::clone(&seen_on_disable);
    let first = ScriptedPlugin::new("first")
        .on_enable(move |ctx| {
            *enable_slot.lock().unwrap() = Some(ctx.plugin_registry_handle());
        })
        .on_disable(move || {
            let registry = disable_slot.lock().unwrap().clone().unwrap();
            *seen.lock().unwrap() = Some(ids(registry.as_ref()));
        });

    let proxy = TestProxy::builder()
        .plugin(first)
        .plugin(ScriptedPlugin::new("second").depends_on("first"))
        .start()
        .await
        .unwrap();
    let registry = registry_slot.lock().unwrap().clone().unwrap();
    assert_eq!(ids(registry.as_ref()), ["first", "second"]);

    proxy.shutdown().await.unwrap();

    assert_eq!(
        *seen_on_disable.lock().unwrap(),
        Some(Vec::new()),
        "second was disabled before first, and first left before its on_disable"
    );
    assert!(ids(registry.as_ref()).is_empty());
}
