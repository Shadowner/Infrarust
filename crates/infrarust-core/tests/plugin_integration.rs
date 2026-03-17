#![allow(clippy::unwrap_used, clippy::expect_used)]

//! End-to-end plugin lifecycle integration test.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use infrarust_api::error::PluginError;
use infrarust_api::event::bus::EventBusExt;
use infrarust_api::event::{BoxFuture, EventPriority};
use infrarust_api::events::lifecycle::PostLoginEvent;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use infrarust_api::types::{GameProfile, PlayerId, ProtocolVersion};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::manager::{PluginManager, PluginServices};
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;

mod mock_services;
use mock_services::{MockBanService, MockConfigService, MockPlayerRegistry};

/// A test plugin that sets a flag when a PostLoginEvent is received.
struct TestPlugin {
    handler_called: Arc<AtomicBool>,
}

impl Plugin for TestPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("test_plugin", "Test Plugin", "1.0.0")
    }

    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        let flag = Arc::clone(&self.handler_called);
        Box::pin(async move {
            ctx.event_bus().subscribe(
                EventPriority::NORMAL,
                move |_event: &mut PostLoginEvent| {
                    flag.store(true, Ordering::SeqCst);
                },
            );
            Ok(())
        })
    }
}

#[tokio::test]
async fn test_plugin_receives_events_end_to_end() {
    let handler_called = Arc::new(AtomicBool::new(false));

    let plugin = TestPlugin {
        handler_called: Arc::clone(&handler_called),
    };

    let event_bus = Arc::new(EventBusImpl::new());

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

    // 1. Create manager and enable
    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(plugin)];
    let mut manager = PluginManager::new(plugins).unwrap();
    let errors = manager.enable_all(&services).await;
    assert!(errors.is_empty());
    assert!(manager.is_plugin_loaded("test_plugin"));

    // 2. Fire a PostLoginEvent
    let event = PostLoginEvent {
        profile: GameProfile {
            uuid: uuid::Uuid::nil(),
            username: "TestPlayer".into(),
            properties: vec![],
        },
        player_id: PlayerId::new(1),
        protocol_version: ProtocolVersion::MINECRAFT_1_21,
    };
    event_bus.fire(event).await;

    // 3. Verify the plugin handler was called
    assert!(
        handler_called.load(Ordering::SeqCst),
        "Plugin handler should have been called on PostLoginEvent"
    );

    // 4. Disable and verify
    manager.disable_all().await;
    assert!(!manager.is_plugin_loaded("test_plugin"));
}

#[tokio::test]
async fn test_dependency_order_end_to_end() {
    let order = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

    struct OrderPlugin {
        meta: PluginMetadata,
        order: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Plugin for OrderPlugin {
        fn metadata(&self) -> PluginMetadata {
            self.meta.clone()
        }
        fn on_enable<'a>(
            &'a self,
            _ctx: &'a dyn PluginContext,
        ) -> BoxFuture<'a, Result<(), PluginError>> {
            Box::pin(async {
                self.order.lock().unwrap().push(self.meta.id.clone());
                Ok(())
            })
        }
    }

    let services = PluginServices {
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
    };

    let plugins: Vec<Box<dyn Plugin>> = vec![
        Box::new(OrderPlugin {
            meta: PluginMetadata::new("child", "Child", "1.0").depends_on("parent"),
            order: Arc::clone(&order),
        }),
        Box::new(OrderPlugin {
            meta: PluginMetadata::new("parent", "Parent", "1.0"),
            order: Arc::clone(&order),
        }),
    ];

    let mut manager = PluginManager::new(plugins).unwrap();
    manager.enable_all(&services).await;

    let enable_order = order.lock().unwrap();
    assert_eq!(*enable_order, vec!["parent", "child"]);
}
