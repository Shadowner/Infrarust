#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::PluginState;
use infrarust_core::plugin::manager::{PluginManager, PluginServices};
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;

mod mock_services;
use mock_services::{MockBanService, MockConfigService, MockPlayerRegistry};

/// A mock plugin that records when on_enable / on_disable are called.
struct MockPlugin {
    metadata: PluginMetadata,
    should_fail: bool,
    on_enable_called: Arc<AtomicBool>,
    on_disable_called: Arc<AtomicBool>,
    enable_order: Arc<AtomicUsize>,
    disable_order: Arc<AtomicUsize>,
    order_counter: Arc<AtomicUsize>,
}

impl MockPlugin {
    fn new(id: &str) -> Self {
        Self {
            metadata: PluginMetadata::new(id, id, "1.0"),
            should_fail: false,
            on_enable_called: Arc::new(AtomicBool::new(false)),
            on_disable_called: Arc::new(AtomicBool::new(false)),
            enable_order: Arc::new(AtomicUsize::new(0)),
            disable_order: Arc::new(AtomicUsize::new(0)),
            order_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn with_metadata(mut self, meta: PluginMetadata) -> Self {
        self.metadata = meta;
        self
    }

    fn failing(mut self) -> Self {
        self.should_fail = true;
        self
    }

    fn with_order_counter(mut self, counter: Arc<AtomicUsize>) -> Self {
        self.order_counter = counter;
        self
    }
}

impl Plugin for MockPlugin {
    fn metadata(&self) -> PluginMetadata {
        self.metadata.clone()
    }

    fn on_enable<'a>(
        &'a self,
        _ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async {
            self.on_enable_called.store(true, Ordering::SeqCst);
            self.enable_order
                .store(self.order_counter.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
            if self.should_fail {
                Err(PluginError::InitFailed("test failure".into()))
            } else {
                Ok(())
            }
        })
    }

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async {
            self.on_disable_called.store(true, Ordering::SeqCst);
            self.disable_order
                .store(self.order_counter.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
            Ok(())
        })
    }
}

fn make_services() -> PluginServices {
    PluginServices {
        event_bus: Arc::new(EventBusImpl::new()),
        player_registry: Arc::new(MockPlayerRegistry),
        server_manager: Arc::new(NoopServerManager),
        ban_service: Arc::new(MockBanService),
        command_manager: Arc::new(CommandManagerImpl::new()),
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(MockConfigService),
        codec_filter_registry: Arc::new(
            infrarust_core::filter::codec_registry::CodecFilterRegistryImpl::new(),
        ),
        transport_filter_registry: Arc::new(
            infrarust_core::filter::transport_registry::TransportFilterRegistryImpl::new(),
        ),
        plugins_dir: PathBuf::from("plugins"),
    }
}

#[tokio::test]
async fn test_enable_all_calls_on_enable() {
    let p1 = MockPlugin::new("a");
    let p2 = MockPlugin::new("b");
    let p1_called = Arc::clone(&p1.on_enable_called);
    let p2_called = Arc::clone(&p2.on_enable_called);

    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(p1), Box::new(p2)];
    let mut manager = PluginManager::new(plugins).unwrap();
    let errors = manager.enable_all(&make_services()).await;

    assert!(errors.is_empty());
    assert!(p1_called.load(Ordering::SeqCst));
    assert!(p2_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_enable_respects_dependency_order() {
    let counter = Arc::new(AtomicUsize::new(0));

    let p_a = MockPlugin::new("a")
        .with_metadata(PluginMetadata::new("a", "A", "1.0").depends_on("b"))
        .with_order_counter(Arc::clone(&counter));
    let p_b = MockPlugin::new("b").with_order_counter(Arc::clone(&counter));

    let order_a = Arc::clone(&p_a.enable_order);
    let order_b = Arc::clone(&p_b.enable_order);

    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(p_a), Box::new(p_b)];
    let mut manager = PluginManager::new(plugins).unwrap();
    manager.enable_all(&make_services()).await;

    assert!(
        order_b.load(Ordering::SeqCst) < order_a.load(Ordering::SeqCst),
        "B should be enabled before A"
    );
}

#[tokio::test]
async fn test_disable_reverse_order() {
    let counter = Arc::new(AtomicUsize::new(0));

    // A depends on B → enable order: B, A → disable order: A, B
    let p_a = MockPlugin::new("a")
        .with_metadata(PluginMetadata::new("a", "A", "1.0").depends_on("b"))
        .with_order_counter(Arc::clone(&counter));
    let p_b = MockPlugin::new("b").with_order_counter(Arc::clone(&counter));

    let disable_order_a = Arc::clone(&p_a.disable_order);
    let disable_order_b = Arc::clone(&p_b.disable_order);

    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(p_a), Box::new(p_b)];
    let mut manager = PluginManager::new(plugins).unwrap();
    manager.enable_all(&make_services()).await;

    // Reset counter for disable ordering
    counter.store(0, Ordering::SeqCst);
    manager.disable_all().await;

    assert!(
        disable_order_a.load(Ordering::SeqCst) < disable_order_b.load(Ordering::SeqCst),
        "A (dependent) must be disabled before B (dependency)"
    );
}

#[tokio::test]
async fn test_failed_plugin_marked_error() {
    let p = MockPlugin::new("fail").failing();
    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(p)];
    let mut manager = PluginManager::new(plugins).unwrap();
    let errors = manager.enable_all(&make_services()).await;

    assert_eq!(errors.len(), 1);
    assert!(matches!(
        manager.plugin_state("fail"),
        Some(PluginState::Error(_))
    ));
}

#[tokio::test]
async fn test_failed_plugin_does_not_block_others() {
    let p_fail = MockPlugin::new("fail").failing();
    let p_ok = MockPlugin::new("ok");
    let ok_called = Arc::clone(&p_ok.on_enable_called);

    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(p_fail), Box::new(p_ok)];
    let mut manager = PluginManager::new(plugins).unwrap();
    let errors = manager.enable_all(&make_services()).await;

    assert_eq!(errors.len(), 1);
    assert!(ok_called.load(Ordering::SeqCst));
    assert!(manager.is_plugin_loaded("ok"));
}

#[tokio::test]
async fn test_is_plugin_loaded() {
    let p = MockPlugin::new("test");
    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(p)];
    let mut manager = PluginManager::new(plugins).unwrap();

    assert!(!manager.is_plugin_loaded("test"));
    manager.enable_all(&make_services()).await;
    assert!(manager.is_plugin_loaded("test"));
    manager.disable_all().await;
    assert!(!manager.is_plugin_loaded("test"));
}

#[tokio::test]
async fn test_cleanup_on_disable() {
    use infrarust_api::event::bus::EventBusExt;
    use infrarust_api::event::EventPriority;
    use infrarust_api::events::proxy::ProxyInitializeEvent;

    let event_bus = Arc::new(EventBusImpl::new());
    let call_count = Arc::new(AtomicUsize::new(0));
    let services = PluginServices {
        event_bus: Arc::clone(&event_bus) as Arc<dyn infrarust_api::event::bus::EventBus>,
        player_registry: Arc::new(MockPlayerRegistry),
        server_manager: Arc::new(NoopServerManager),
        ban_service: Arc::new(MockBanService),
        command_manager: Arc::new(CommandManagerImpl::new()),
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(MockConfigService),
        codec_filter_registry: Arc::new(
            infrarust_core::filter::codec_registry::CodecFilterRegistryImpl::new(),
        ),
        transport_filter_registry: Arc::new(
            infrarust_core::filter::transport_registry::TransportFilterRegistryImpl::new(),
        ),
        plugins_dir: PathBuf::from("plugins"),
    };

    // A plugin that registers a listener which increments a counter
    let counter = Arc::clone(&call_count);
    struct ListenerPlugin {
        counter: Arc<AtomicUsize>,
    }
    impl Plugin for ListenerPlugin {
        fn metadata(&self) -> PluginMetadata {
            PluginMetadata::new("listener", "L", "1.0")
        }
        fn on_enable<'a>(
            &'a self,
            ctx: &'a dyn PluginContext,
        ) -> BoxFuture<'a, Result<(), PluginError>> {
            let counter = Arc::clone(&self.counter);
            Box::pin(async move {
                ctx.event_bus().subscribe(
                    EventPriority::NORMAL,
                    move |_event: &mut ProxyInitializeEvent| {
                        counter.fetch_add(1, Ordering::SeqCst);
                    },
                );
                Ok(())
            })
        }
    }

    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(ListenerPlugin { counter })];
    let mut manager = PluginManager::new(plugins).unwrap();
    manager.enable_all(&services).await;

    // Fire event — handler should be called
    event_bus.fire(ProxyInitializeEvent).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1, "Handler should be called once");

    // Disable plugin — cleanup removes listener
    manager.disable_all().await;

    // Fire event again — handler should NOT be called
    event_bus.fire(ProxyInitializeEvent).await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "Handler should not be called after cleanup"
    );
}

#[tokio::test]
async fn test_list_plugins() {
    let p1 = MockPlugin::new("alpha");
    let p2 = MockPlugin::new("beta");
    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(p1), Box::new(p2)];
    let manager = PluginManager::new(plugins).unwrap();

    let list = manager.list_plugins();
    assert_eq!(list.len(), 2);
    let ids: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"alpha"));
    assert!(ids.contains(&"beta"));
}
