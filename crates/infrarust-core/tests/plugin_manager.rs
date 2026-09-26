#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::manager::{PluginManager, PluginServices};
use infrarust_core::plugin::static_loader::StaticPluginLoader;
use infrarust_core::plugin::{PluginContextFactoryImpl, PluginState};
use infrarust_core::test_support::TestPlugin;

fn registered(loader: &StaticPluginLoader, plugin: TestPlugin) -> TestPlugin {
    plugin.register(loader);
    plugin
}

fn factory() -> Arc<PluginContextFactoryImpl> {
    Arc::new(PluginContextFactoryImpl::new(
        PluginServices::for_tests(),
        HashMap::new(),
    ))
}

#[tokio::test]
async fn test_enable_all_calls_on_enable() {
    let loader = StaticPluginLoader::new();
    let a = registered(&loader, TestPlugin::new("a"));
    let b = registered(&loader, TestPlugin::new("b"));

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    let errors = manager.load_and_enable_all(factory()).await;

    assert!(errors.is_empty());
    assert_eq!(a.enable_calls(), 1);
    assert_eq!(b.enable_calls(), 1);
}

#[tokio::test]
async fn test_config_disabled_plugin_is_skipped() {
    let loader = StaticPluginLoader::new();
    let on = registered(&loader, TestPlugin::new("on"));
    let off = registered(&loader, TestPlugin::new("off"));

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.set_disabled_plugins(std::collections::HashSet::from(["off".to_string()]));
    manager.discover_all(Path::new("plugins")).await.unwrap();
    let errors = manager.load_and_enable_all(factory()).await;

    assert!(errors.is_empty());
    assert!(on.is_enabled());
    assert!(!off.is_enabled());
    assert!(matches!(
        manager.plugin_state("off"),
        Some(PluginState::Disabled)
    ));
}

#[tokio::test]
async fn test_enable_respects_dependency_order() {
    let loader = StaticPluginLoader::new();
    let counter = Arc::new(AtomicUsize::new(0));
    let a = registered(
        &loader,
        TestPlugin::new("a").depends_on("b").sharing_order(&counter),
    );
    let b = registered(&loader, TestPlugin::new("b").sharing_order(&counter));

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    manager.load_and_enable_all(factory()).await;

    assert!(
        b.enable_order() < a.enable_order(),
        "B should be enabled before A"
    );
}

#[tokio::test]
async fn test_disable_reverse_order() {
    let loader = StaticPluginLoader::new();
    let counter = Arc::new(AtomicUsize::new(0));
    let a = registered(
        &loader,
        TestPlugin::new("a").depends_on("b").sharing_order(&counter),
    );
    let b = registered(&loader, TestPlugin::new("b").sharing_order(&counter));

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    manager.load_and_enable_all(factory()).await;

    counter.store(0, Ordering::SeqCst);
    manager.shutdown().await;

    assert!(
        a.disable_order() < b.disable_order(),
        "A (dependent) must be disabled before B (dependency)"
    );
}

#[tokio::test]
async fn test_failed_plugin_marked_error() {
    let loader = StaticPluginLoader::new();
    registered(&loader, TestPlugin::new("fail").fail_on_enable());

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    let errors = manager.load_and_enable_all(factory()).await;

    assert_eq!(errors.len(), 1);
    assert!(matches!(
        manager.plugin_state("fail"),
        Some(PluginState::Error(_))
    ));
}

#[tokio::test]
async fn test_failed_plugin_does_not_block_others() {
    let loader = StaticPluginLoader::new();
    registered(&loader, TestPlugin::new("fail").fail_on_enable());
    let ok = registered(&loader, TestPlugin::new("ok"));

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    let errors = manager.load_and_enable_all(factory()).await;

    assert_eq!(errors.len(), 1);
    assert!(ok.is_enabled());
    assert!(manager.is_plugin_loaded("ok"));
}

#[tokio::test]
async fn test_is_plugin_loaded() {
    let loader = StaticPluginLoader::new();
    registered(&loader, TestPlugin::new("test"));

    let mut manager = PluginManager::new(vec![Box::new(loader)]);

    assert!(!manager.is_plugin_loaded("test"));

    manager.discover_all(Path::new("plugins")).await.unwrap();
    manager.load_and_enable_all(factory()).await;
    assert!(manager.is_plugin_loaded("test"));

    manager.shutdown().await;
    assert!(!manager.is_plugin_loaded("test"));
}

#[tokio::test]
async fn test_cleanup_on_disable() {
    use infrarust_api::event::EventPriority;
    use infrarust_api::event::bus::EventBusExt;
    use infrarust_api::events::proxy::ProxyInitializeEvent;

    let event_bus = Arc::new(EventBusImpl::new());
    let call_count = Arc::new(AtomicUsize::new(0));
    let factory = Arc::new(PluginContextFactoryImpl::new(
        PluginServices::for_tests_with(Arc::clone(&event_bus)),
        HashMap::new(),
    ));

    let counter = Arc::clone(&call_count);
    let loader = StaticPluginLoader::new();
    registered(
        &loader,
        TestPlugin::new("listener").on_enable(move |ctx| {
            let counter = Arc::clone(&counter);
            ctx.event_bus().subscribe(
                EventPriority::NORMAL,
                move |_event: &mut ProxyInitializeEvent| {
                    counter.fetch_add(1, Ordering::SeqCst);
                },
            );
        }),
    );

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    manager.load_and_enable_all(factory).await;

    event_bus.fire(ProxyInitializeEvent).await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "Handler should be called once"
    );

    manager.shutdown().await;

    event_bus.fire(ProxyInitializeEvent).await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "Handler should not be called after cleanup"
    );
}

#[tokio::test]
async fn test_list_plugins() {
    let loader = StaticPluginLoader::new();
    registered(&loader, TestPlugin::new("alpha"));
    registered(&loader, TestPlugin::new("beta"));

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    manager.load_and_enable_all(factory()).await;

    let list = manager.list_plugins();
    assert_eq!(list.len(), 2);
    let ids: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"alpha"));
    assert!(ids.contains(&"beta"));
}

#[tokio::test]
async fn lifecycle_events_follow_enable_and_disable_order() {
    use infrarust_api::event::EventPriority;
    use infrarust_api::event::bus::{EventBus, EventBusExt};
    use infrarust_api::events::plugin::{PluginDisabledEvent, PluginEnabledEvent};

    let bus = Arc::new(EventBusImpl::new());
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let listen: &dyn EventBus = bus.as_ref();
    let on_enabled = Arc::clone(&seen);
    listen.subscribe(EventPriority::NORMAL, move |e: &mut PluginEnabledEvent| {
        on_enabled
            .lock()
            .unwrap()
            .push(format!("+{}@{}", e.plugin_id, e.version));
    });
    let on_disabled = Arc::clone(&seen);
    listen.subscribe(EventPriority::NORMAL, move |e: &mut PluginDisabledEvent| {
        on_disabled
            .lock()
            .unwrap()
            .push(format!("-{}", e.plugin_id));
    });

    let loader = StaticPluginLoader::new();
    registered(
        &loader,
        TestPlugin::new("addon").version("2.0.0").depends_on("base"),
    );
    registered(&loader, TestPlugin::new("base"));
    registered(&loader, TestPlugin::new("broken").fail_on_enable());
    registered(&loader, TestPlugin::new("extra"));

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.set_event_bus(Arc::clone(&bus));
    manager.discover_all(Path::new("plugins")).await.unwrap();
    let errors = manager.load_and_enable_all(factory()).await;
    assert_eq!(errors.len(), 1);
    assert_eq!(
        *seen.lock().unwrap(),
        ["+base@1.0.0", "+addon@2.0.0", "+extra@1.0.0"]
    );

    assert!(manager.disable_plugin("base").await.is_err());
    assert!(manager.disable_plugin("broken").await.is_err());
    manager.disable_plugin("extra").await.unwrap();
    assert!(!manager.is_plugin_loaded("extra"));
    assert!(matches!(
        manager.plugin_state("extra"),
        Some(PluginState::Disabled)
    ));
    assert!(manager.plugin_context("extra").is_none());

    manager.shutdown().await;
    assert_eq!(
        *seen.lock().unwrap(),
        [
            "+base@1.0.0",
            "+addon@2.0.0",
            "+extra@1.0.0",
            "-extra",
            "-addon",
            "-base"
        ]
    );
}

#[tokio::test]
async fn disabling_a_plugin_evicts_its_context_from_the_factory() {
    let loader = StaticPluginLoader::new();
    registered(&loader, TestPlugin::new("gone"));
    registered(&loader, TestPlugin::new("stays"));
    let factory = factory();

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    manager.load_and_enable_all(Arc::clone(&factory)).await;
    let held = factory.context("gone");
    assert!(factory.remembers_context("gone"));

    manager.disable_plugin("gone").await.unwrap();
    assert!(!factory.remembers_context("gone"));
    assert!(factory.remembers_context("stays"));

    manager.shutdown().await;
    assert!(!factory.remembers_context("stays"));
    drop(held);
}

#[tokio::test]
async fn a_plugin_that_fails_to_enable_leaves_no_context_behind() {
    let loader = StaticPluginLoader::new();
    registered(&loader, TestPlugin::new("broken").fail_on_enable());
    let factory = factory();

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    let errors = manager.load_and_enable_all(Arc::clone(&factory)).await;

    assert_eq!(errors.len(), 1);
    assert!(!factory.remembers_context("broken"));
}
