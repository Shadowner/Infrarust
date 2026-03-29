#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::PluginState;
use infrarust_core::plugin::context_factory::PluginContextFactory;
use infrarust_core::plugin::manager::{PluginManager, PluginServices};
use infrarust_core::plugin::static_loader::StaticPluginLoader;
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;

mod mock_services;
use mock_services::{MockBanService, MockConfigService, MockPlayerRegistry};

struct MockPluginContext {
    plugin_id: String,
}

impl infrarust_api::plugin::private::Sealed for MockPluginContext {}

impl PluginContext for MockPluginContext {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn event_bus(&self) -> &dyn infrarust_api::event::bus::EventBus {
        unimplemented!("mock")
    }
    fn player_registry(&self) -> &dyn infrarust_api::services::player_registry::PlayerRegistry {
        unimplemented!("mock")
    }
    fn player_registry_handle(
        &self,
    ) -> Arc<dyn infrarust_api::services::player_registry::PlayerRegistry> {
        unimplemented!("mock")
    }
    fn server_manager(&self) -> &dyn infrarust_api::services::server_manager::ServerManager {
        unimplemented!("mock")
    }
    fn ban_service(&self) -> &dyn infrarust_api::services::ban_service::BanService {
        unimplemented!("mock")
    }
    fn config_service(&self) -> &dyn infrarust_api::services::config_service::ConfigService {
        unimplemented!("mock")
    }
    fn command_manager(&self) -> &dyn infrarust_api::command::CommandManager {
        unimplemented!("mock")
    }
    fn scheduler(&self) -> &dyn infrarust_api::services::scheduler::Scheduler {
        unimplemented!("mock")
    }
    fn register_limbo_handler(&self, _handler: Box<dyn infrarust_api::limbo::LimboHandler>) {
        unimplemented!("mock")
    }
    fn register_config_provider(
        &self,
        _provider: Box<dyn infrarust_api::provider::PluginConfigProvider>,
    ) {
        // no-op for tests
    }
    fn codec_filters(&self) -> Option<&dyn infrarust_api::filter::registry::CodecFilterRegistry> {
        None
    }
    fn transport_filters(
        &self,
    ) -> Option<&dyn infrarust_api::filter::registry::TransportFilterRegistry> {
        None
    }
    fn plugin_id(&self) -> &str {
        &self.plugin_id
    }
    fn data_dir(&self) -> std::path::PathBuf {
        std::path::PathBuf::from("plugins").join(&self.plugin_id)
    }
    fn plugin_registry(&self) -> &dyn infrarust_api::services::plugin_registry::PluginRegistry {
        unimplemented!("mock")
    }
    fn plugin_registry_handle(
        &self,
    ) -> Arc<dyn infrarust_api::services::plugin_registry::PluginRegistry> {
        unimplemented!("mock")
    }
    fn server_manager_handle(
        &self,
    ) -> Arc<dyn infrarust_api::services::server_manager::ServerManager> {
        unimplemented!("mock")
    }
    fn ban_service_handle(&self) -> Arc<dyn infrarust_api::services::ban_service::BanService> {
        unimplemented!("mock")
    }
    fn config_service_handle(
        &self,
    ) -> Arc<dyn infrarust_api::services::config_service::ConfigService> {
        unimplemented!("mock")
    }
    fn event_bus_handle(&self) -> Arc<dyn infrarust_api::event::bus::EventBus> {
        unimplemented!("mock")
    }
    fn proxy_shutdown(&self) -> tokio_util::sync::CancellationToken {
        tokio_util::sync::CancellationToken::new()
    }
}

struct MockPluginContextFactory;

impl PluginContextFactory for MockPluginContextFactory {
    fn create_context(&self, plugin_id: &str) -> Arc<dyn PluginContext> {
        Arc::new(MockPluginContext {
            plugin_id: plugin_id.to_string(),
        })
    }
}

fn make_real_factory(services: PluginServices) -> infrarust_core::plugin::PluginContextFactoryImpl {
    infrarust_core::plugin::PluginContextFactoryImpl::new(
        services,
        std::collections::HashMap::new(),
    )
}

#[allow(dead_code)]
fn make_services() -> PluginServices {
    PluginServices {
        event_bus: Arc::new(EventBusImpl::new()),
        player_registry: Arc::new(MockPlayerRegistry),
        server_manager: Arc::new(NoopServerManager),
        ban_service: Arc::new(MockBanService),
        command_manager: Arc::new(CommandManagerImpl::new()),
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(MockConfigService),
        plugin_registry: Arc::new(infrarust_core::plugin::PluginRegistryImpl::new()),
        codec_filter_registry: Arc::new(
            infrarust_core::filter::codec_registry::CodecFilterRegistryImpl::new(),
        ),
        transport_filter_registry: Arc::new(
            infrarust_core::filter::transport_registry::TransportFilterRegistryImpl::new(),
        ),
        domain_router: Arc::new(infrarust_core::routing::DomainRouter::new()),
        proxy_shutdown: tokio_util::sync::CancellationToken::new(),
        plugins_dir: PathBuf::from("plugins"),
    }
}

struct MockPlugin {
    metadata: PluginMetadata,
    should_fail: bool,
    on_enable_called: Arc<AtomicBool>,
    on_disable_called: Arc<AtomicBool>,
    enable_order: Arc<AtomicUsize>,
    disable_order: Arc<AtomicUsize>,
    order_counter: Arc<AtomicUsize>,
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
            self.enable_order.store(
                self.order_counter.fetch_add(1, Ordering::SeqCst),
                Ordering::SeqCst,
            );
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
            self.disable_order.store(
                self.order_counter.fetch_add(1, Ordering::SeqCst),
                Ordering::SeqCst,
            );
            Ok(())
        })
    }
}

#[allow(dead_code)]
struct MockPluginState {
    on_enable_called: Arc<AtomicBool>,
    on_disable_called: Arc<AtomicBool>,
    enable_order: Arc<AtomicUsize>,
    disable_order: Arc<AtomicUsize>,
    order_counter: Arc<AtomicUsize>,
}

fn register_mock(
    loader: &StaticPluginLoader,
    meta: PluginMetadata,
    should_fail: bool,
    order_counter: Arc<AtomicUsize>,
) -> MockPluginState {
    let on_enable_called = Arc::new(AtomicBool::new(false));
    let on_disable_called = Arc::new(AtomicBool::new(false));
    let enable_order = Arc::new(AtomicUsize::new(0));
    let disable_order = Arc::new(AtomicUsize::new(0));

    let state = MockPluginState {
        on_enable_called: on_enable_called.clone(),
        on_disable_called: on_disable_called.clone(),
        enable_order: enable_order.clone(),
        disable_order: disable_order.clone(),
        order_counter: order_counter.clone(),
    };

    let m = meta.clone();
    loader.register(meta, move || {
        Box::new(MockPlugin {
            metadata: m.clone(),
            should_fail,
            on_enable_called: on_enable_called.clone(),
            on_disable_called: on_disable_called.clone(),
            enable_order: enable_order.clone(),
            disable_order: disable_order.clone(),
            order_counter: order_counter.clone(),
        })
    });

    state
}

#[tokio::test]
async fn test_enable_all_calls_on_enable() {
    let loader = StaticPluginLoader::new();
    let counter = Arc::new(AtomicUsize::new(0));

    let s1 = register_mock(
        &loader,
        PluginMetadata::new("a", "a", "1.0"),
        false,
        counter.clone(),
    );
    let s2 = register_mock(
        &loader,
        PluginMetadata::new("b", "b", "1.0"),
        false,
        counter,
    );

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    let errors = manager.load_and_enable_all(&MockPluginContextFactory).await;

    assert!(errors.is_empty());
    assert!(s1.on_enable_called.load(Ordering::SeqCst));
    assert!(s2.on_enable_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_enable_respects_dependency_order() {
    let loader = StaticPluginLoader::new();
    let counter = Arc::new(AtomicUsize::new(0));

    let s_a = register_mock(
        &loader,
        PluginMetadata::new("a", "A", "1.0").depends_on("b"),
        false,
        counter.clone(),
    );
    let s_b = register_mock(
        &loader,
        PluginMetadata::new("b", "B", "1.0"),
        false,
        counter,
    );

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    manager.load_and_enable_all(&MockPluginContextFactory).await;

    assert!(
        s_b.enable_order.load(Ordering::SeqCst) < s_a.enable_order.load(Ordering::SeqCst),
        "B should be enabled before A"
    );
}

#[tokio::test]
async fn test_disable_reverse_order() {
    let loader = StaticPluginLoader::new();
    let counter = Arc::new(AtomicUsize::new(0));

    let s_a = register_mock(
        &loader,
        PluginMetadata::new("a", "A", "1.0").depends_on("b"),
        false,
        counter.clone(),
    );
    let s_b = register_mock(
        &loader,
        PluginMetadata::new("b", "B", "1.0"),
        false,
        counter.clone(),
    );

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    manager.load_and_enable_all(&MockPluginContextFactory).await;

    // Reset counter for disable ordering
    counter.store(0, Ordering::SeqCst);
    manager.shutdown().await;

    assert!(
        s_a.disable_order.load(Ordering::SeqCst) < s_b.disable_order.load(Ordering::SeqCst),
        "A (dependent) must be disabled before B (dependency)"
    );
}

#[tokio::test]
async fn test_failed_plugin_marked_error() {
    let loader = StaticPluginLoader::new();
    let counter = Arc::new(AtomicUsize::new(0));

    register_mock(
        &loader,
        PluginMetadata::new("fail", "fail", "1.0"),
        true,
        counter,
    );

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    let errors = manager.load_and_enable_all(&MockPluginContextFactory).await;

    assert_eq!(errors.len(), 1);
    assert!(matches!(
        manager.plugin_state("fail"),
        Some(PluginState::Error(_))
    ));
}

#[tokio::test]
async fn test_failed_plugin_does_not_block_others() {
    let loader = StaticPluginLoader::new();
    let counter = Arc::new(AtomicUsize::new(0));

    register_mock(
        &loader,
        PluginMetadata::new("fail", "fail", "1.0"),
        true,
        counter.clone(),
    );
    let s_ok = register_mock(
        &loader,
        PluginMetadata::new("ok", "ok", "1.0"),
        false,
        counter,
    );

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    let errors = manager.load_and_enable_all(&MockPluginContextFactory).await;

    assert_eq!(errors.len(), 1);
    assert!(s_ok.on_enable_called.load(Ordering::SeqCst));
    assert!(manager.is_plugin_loaded("ok"));
}

#[tokio::test]
async fn test_is_plugin_loaded() {
    let loader = StaticPluginLoader::new();
    let counter = Arc::new(AtomicUsize::new(0));

    register_mock(
        &loader,
        PluginMetadata::new("test", "test", "1.0"),
        false,
        counter,
    );

    let mut manager = PluginManager::new(vec![Box::new(loader)]);

    assert!(!manager.is_plugin_loaded("test"));

    manager.discover_all(Path::new("plugins")).await.unwrap();
    manager.load_and_enable_all(&MockPluginContextFactory).await;
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

    let services = PluginServices {
        event_bus: Arc::clone(&event_bus) as Arc<dyn infrarust_api::event::bus::EventBus>,
        player_registry: Arc::new(MockPlayerRegistry),
        server_manager: Arc::new(NoopServerManager),
        ban_service: Arc::new(MockBanService),
        command_manager: Arc::new(CommandManagerImpl::new()),
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(MockConfigService),
        plugin_registry: Arc::new(infrarust_core::plugin::PluginRegistryImpl::new()),
        codec_filter_registry: Arc::new(
            infrarust_core::filter::codec_registry::CodecFilterRegistryImpl::new(),
        ),
        transport_filter_registry: Arc::new(
            infrarust_core::filter::transport_registry::TransportFilterRegistryImpl::new(),
        ),
        domain_router: Arc::new(infrarust_core::routing::DomainRouter::new()),
        proxy_shutdown: tokio_util::sync::CancellationToken::new(),
        plugins_dir: PathBuf::from("plugins"),
    };

    let factory = make_real_factory(services);

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

    let loader = StaticPluginLoader::new();
    let counter_for_factory = counter.clone();
    loader.register(PluginMetadata::new("listener", "L", "1.0"), move || {
        Box::new(ListenerPlugin {
            counter: counter_for_factory.clone(),
        })
    });

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    manager.load_and_enable_all(&factory).await;

    // Fire event — handler should be called
    event_bus.fire(ProxyInitializeEvent).await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "Handler should be called once"
    );

    // Shutdown — cleanup removes listener
    manager.shutdown().await;

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
    let loader = StaticPluginLoader::new();
    let counter = Arc::new(AtomicUsize::new(0));

    register_mock(
        &loader,
        PluginMetadata::new("alpha", "alpha", "1.0"),
        false,
        counter.clone(),
    );
    register_mock(
        &loader,
        PluginMetadata::new("beta", "beta", "1.0"),
        false,
        counter,
    );

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    manager.load_and_enable_all(&MockPluginContextFactory).await;

    let list = manager.list_plugins();
    assert_eq!(list.len(), 2);
    let ids: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"alpha"));
    assert!(ids.contains(&"beta"));
}
