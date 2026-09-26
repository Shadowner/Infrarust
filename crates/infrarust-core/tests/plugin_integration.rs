#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use infrarust_api::event::EventPriority;
use infrarust_api::event::bus::EventBusExt;
use infrarust_api::events::lifecycle::PostLoginEvent;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::player::PlayerSession;
use infrarust_core::plugin::PluginContextFactoryImpl;
use infrarust_core::plugin::manager::{PluginManager, PluginServices};
use infrarust_core::plugin::static_loader::StaticPluginLoader;
use infrarust_core::test_support::TestPlugin;

#[tokio::test]
async fn test_plugin_receives_events_end_to_end() {
    let handler_called = Arc::new(AtomicBool::new(false));
    let event_bus = Arc::new(EventBusImpl::new());
    let factory = PluginContextFactoryImpl::new(
        PluginServices::for_tests_with(Arc::clone(&event_bus)),
        HashMap::new(),
    );

    let flag = Arc::clone(&handler_called);
    let loader = StaticPluginLoader::new();
    TestPlugin::new("test_plugin")
        .on_enable(move |ctx| {
            let flag = Arc::clone(&flag);
            ctx.event_bus()
                .subscribe(EventPriority::NORMAL, move |_event: &mut PostLoginEvent| {
                    flag.store(true, Ordering::SeqCst);
                });
        })
        .register(&loader);

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    let errors = manager.load_and_enable_all(Arc::new(factory)).await;
    assert!(errors.is_empty());
    assert!(manager.is_plugin_loaded("test_plugin"));

    let (player, _commands) = PlayerSession::new_test(true);
    let event = PostLoginEvent::new(player);
    event_bus.fire(event).await;

    assert!(
        handler_called.load(Ordering::SeqCst),
        "Plugin handler should have been called on PostLoginEvent"
    );

    manager.shutdown().await;
    assert!(!manager.is_plugin_loaded("test_plugin"));
}

#[tokio::test]
async fn test_dependency_order_end_to_end() {
    let factory = PluginContextFactoryImpl::new(PluginServices::for_tests(), HashMap::new());
    let order = Arc::new(AtomicUsize::new(0));
    let child = TestPlugin::new("child")
        .depends_on("parent")
        .sharing_order(&order);
    let parent = TestPlugin::new("parent").sharing_order(&order);

    let loader = StaticPluginLoader::new();
    child.register(&loader);
    parent.register(&loader);

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    manager.load_and_enable_all(Arc::new(factory)).await;

    assert!(parent.enable_order() < child.enable_order());
}
