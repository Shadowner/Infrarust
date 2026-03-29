---
title: Testing Plugins
description: Write unit and integration tests for Infrarust plugins using mock services, the static loader, and the plugin manager.
outline: [2, 3]
---

# Testing Plugins

Infrarust's plugin system is built on trait objects, which makes testing straightforward. You can mock individual services, wire up a real event bus, or run a full plugin lifecycle through the `PluginManager`.

This page covers three levels of testing, from isolated unit tests to end-to-end integration tests.

## Mock services

The `PluginContext` trait is sealed (only the proxy implements it), but every service it exposes is a trait you can mock independently. Infrarust's own test suite includes no-op mocks for the three services that require storage backends.

### MockPlayerRegistry

Returns empty results for all lookups:

```rust
use std::sync::Arc;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{PlayerId, ServerId};

pub struct MockPlayerRegistry;

impl infrarust_api::services::player_registry::private::Sealed
    for MockPlayerRegistry {}

impl PlayerRegistry for MockPlayerRegistry {
    fn get_player(
        &self, _username: &str,
    ) -> Option<Arc<dyn infrarust_api::player::Player>> {
        None
    }
    fn get_player_by_uuid(
        &self, _uuid: &uuid::Uuid,
    ) -> Option<Arc<dyn infrarust_api::player::Player>> {
        None
    }
    fn get_player_by_id(
        &self, _id: PlayerId,
    ) -> Option<Arc<dyn infrarust_api::player::Player>> {
        None
    }
    fn get_players_on_server(
        &self, _server: &ServerId,
    ) -> Vec<Arc<dyn infrarust_api::player::Player>> {
        vec![]
    }
    fn get_all_players(
        &self,
    ) -> Vec<Arc<dyn infrarust_api::player::Player>> {
        vec![]
    }
    fn online_count(&self) -> usize { 0 }
    fn online_count_on(&self, _server: &ServerId) -> usize { 0 }
}
```

### MockBanService

All operations succeed and report no bans:

```rust
use infrarust_api::error::ServiceError;
use infrarust_api::event::BoxFuture;
use infrarust_api::services::ban_service::{BanEntry, BanTarget};
use std::time::Duration;

pub struct MockBanService;

impl infrarust_api::services::ban_service::private::Sealed
    for MockBanService {}

impl infrarust_api::services::ban_service::BanService for MockBanService {
    fn ban(
        &self, _target: BanTarget, _reason: Option<String>,
        _duration: Option<Duration>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async { Ok(()) })
    }
    fn unban(
        &self, _target: &BanTarget,
    ) -> BoxFuture<'_, Result<bool, ServiceError>> {
        Box::pin(async { Ok(false) })
    }
    fn is_banned(
        &self, _target: &BanTarget,
    ) -> BoxFuture<'_, Result<bool, ServiceError>> {
        Box::pin(async { Ok(false) })
    }
    fn get_ban(
        &self, _target: &BanTarget,
    ) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        Box::pin(async { Ok(None) })
    }
    fn get_all_bans(
        &self,
    ) -> BoxFuture<'_, Result<Vec<BanEntry>, ServiceError>> {
        Box::pin(async { Ok(vec![]) })
    }
}
```

### MockConfigService

Returns `None` for all config lookups:

```rust
use infrarust_api::services::config_service::ServerConfig;
use infrarust_api::types::ServerId;

pub struct MockConfigService;

impl infrarust_api::services::config_service::private::Sealed
    for MockConfigService {}

impl infrarust_api::services::config_service::ConfigService
    for MockConfigService
{
    fn get_server_config(
        &self, _server: &ServerId,
    ) -> Option<ServerConfig> {
        None
    }
    fn get_all_server_configs(&self) -> Vec<ServerConfig> { vec![] }
    fn get_value(&self, _key: &str) -> Option<String> { None }
}
```

### MockPluginContext

When you need to test `on_enable` in isolation without a real `PluginManager`, you can implement `PluginContext` directly. Stub methods you don't need with `unimplemented!("mock")` so your test panics if the plugin calls something unexpected:

```rust
use std::sync::Arc;
use infrarust_api::plugin::PluginContext;

struct MockPluginContext {
    plugin_id: String,
}

impl infrarust_api::plugin::private::Sealed for MockPluginContext {}

impl PluginContext for MockPluginContext {
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn plugin_id(&self) -> &str { &self.plugin_id }
    fn data_dir(&self) -> std::path::PathBuf {
        std::path::PathBuf::from("plugins").join(&self.plugin_id)
    }

    fn event_bus(&self) -> &dyn infrarust_api::event::bus::EventBus {
        unimplemented!("mock")
    }
    fn player_registry(
        &self,
    ) -> &dyn infrarust_api::services::player_registry::PlayerRegistry {
        unimplemented!("mock")
    }
    fn player_registry_handle(
        &self,
    ) -> Arc<dyn infrarust_api::services::player_registry::PlayerRegistry> {
        unimplemented!("mock")
    }
    fn server_manager(
        &self,
    ) -> &dyn infrarust_api::services::server_manager::ServerManager {
        unimplemented!("mock")
    }
    fn server_manager_handle(
        &self,
    ) -> Arc<dyn infrarust_api::services::server_manager::ServerManager> {
        unimplemented!("mock")
    }
    fn ban_service(
        &self,
    ) -> &dyn infrarust_api::services::ban_service::BanService {
        unimplemented!("mock")
    }
    fn ban_service_handle(
        &self,
    ) -> Arc<dyn infrarust_api::services::ban_service::BanService> {
        unimplemented!("mock")
    }
    fn config_service(
        &self,
    ) -> &dyn infrarust_api::services::config_service::ConfigService {
        unimplemented!("mock")
    }
    fn config_service_handle(
        &self,
    ) -> Arc<dyn infrarust_api::services::config_service::ConfigService> {
        unimplemented!("mock")
    }
    fn command_manager(
        &self,
    ) -> &dyn infrarust_api::command::CommandManager {
        unimplemented!("mock")
    }
    fn scheduler(
        &self,
    ) -> &dyn infrarust_api::services::scheduler::Scheduler {
        unimplemented!("mock")
    }
    fn event_bus_handle(
        &self,
    ) -> Arc<dyn infrarust_api::event::bus::EventBus> {
        unimplemented!("mock")
    }
    fn register_limbo_handler(
        &self, _handler: Box<dyn infrarust_api::limbo::LimboHandler>,
    ) {
        unimplemented!("mock")
    }
    fn register_config_provider(
        &self,
        _provider: Box<dyn infrarust_api::provider::PluginConfigProvider>,
    ) {
        // no-op for tests
    }
    fn codec_filters(
        &self,
    ) -> Option<&dyn infrarust_api::filter::registry::CodecFilterRegistry> {
        None
    }
    fn transport_filters(
        &self,
    ) -> Option<&dyn infrarust_api::filter::registry::TransportFilterRegistry> {
        None
    }
}
```

You can also wrap this in a factory so the `PluginManager` can create per-plugin contexts:

```rust
use infrarust_core::plugin::context_factory::PluginContextFactory;

struct MockPluginContextFactory;

impl PluginContextFactory for MockPluginContextFactory {
    fn create_context(
        &self, plugin_id: &str,
    ) -> Arc<dyn PluginContext> {
        Arc::new(MockPluginContext {
            plugin_id: plugin_id.to_string(),
        })
    }
}
```

## Unit testing a plugin

A unit test creates your plugin, calls `on_enable` with a mock context, and checks the result. Use `Arc<AtomicBool>` flags to verify that callbacks fire.

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};

struct MyPlugin {
    enabled: Arc<AtomicBool>,
}

impl Plugin for MyPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("my_plugin", "My Plugin", "0.1.0")
    }

    fn on_enable<'a>(
        &'a self, _ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        self.enabled.store(true, Ordering::Relaxed);
        Box::pin(async { Ok(()) })
    }

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        self.enabled.store(false, Ordering::Relaxed);
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn test_enable_sets_flag() {
    let enabled = Arc::new(AtomicBool::new(false));
    let plugin = MyPlugin { enabled: enabled.clone() };

    let ctx = Arc::new(MockPluginContext {
        plugin_id: "my_plugin".into(),
    });

    plugin.on_enable(ctx.as_ref()).await.unwrap();
    assert!(enabled.load(Ordering::Relaxed));

    plugin.on_disable().await.unwrap();
    assert!(!enabled.load(Ordering::Relaxed));
}
```

### Testing failure paths

Return `PluginError::InitFailed` from `on_enable` to test error handling:

```rust
struct FailingPlugin;

impl Plugin for FailingPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("fail", "Failing", "1.0")
    }
    fn on_enable<'a>(
        &'a self, _ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async {
            Err(PluginError::InitFailed("database unreachable".into()))
        })
    }
}

#[tokio::test]
async fn test_enable_returns_error() {
    let plugin = FailingPlugin;
    let ctx = Arc::new(MockPluginContext {
        plugin_id: "fail".into(),
    });

    let result = plugin.on_enable(ctx.as_ref()).await;
    assert!(matches!(result, Err(PluginError::InitFailed(_))));
}
```

## Integration testing with real services

For tests that need real event dispatch, command registration, or scheduling, use the actual `infrarust-core` implementations alongside your mocks.

### Building PluginServices

`PluginServices` holds every service the proxy passes to plugins. You can mix real and mock implementations:

```rust
use std::path::PathBuf;
use std::collections::HashMap;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::PluginContextFactoryImpl;
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;

let event_bus = Arc::new(EventBusImpl::new());

let services = PluginServices {
    event_bus: Arc::clone(&event_bus)
        as Arc<dyn infrarust_api::event::bus::EventBus>,
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
    domain_router: Arc::new(
        infrarust_core::routing::DomainRouter::new(),
    ),
    plugins_dir: PathBuf::from("plugins"),
};

let factory = PluginContextFactoryImpl::new(
    services, HashMap::new(),
);
```

The `EventBusImpl`, `CommandManagerImpl`, and `SchedulerImpl` are real implementations that work without any proxy infrastructure. `NoopServerManager` is a built-in stub for proxies without managed servers.

### Testing event handling

Register a plugin, fire an event, and assert the handler was called:

```rust
use infrarust_api::event::bus::EventBusExt;
use infrarust_api::event::EventPriority;
use infrarust_api::events::lifecycle::PostLoginEvent;
use infrarust_api::types::{GameProfile, PlayerId, ProtocolVersion};
use infrarust_core::plugin::manager::PluginManager;
use infrarust_core::plugin::static_loader::StaticPluginLoader;

struct EventTestPlugin {
    handler_called: Arc<AtomicBool>,
}

impl Plugin for EventTestPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("event_test", "Event Test", "1.0.0")
    }
    fn on_enable<'a>(
        &'a self, ctx: &'a dyn PluginContext,
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
async fn test_plugin_receives_post_login() {
    let handler_called = Arc::new(AtomicBool::new(false));

    // ... build PluginServices and factory as shown above ...

    let loader = StaticPluginLoader::new();
    let flag = handler_called.clone();
    loader.register(
        PluginMetadata::new("event_test", "Event Test", "1.0.0"),
        move || Box::new(EventTestPlugin {
            handler_called: flag.clone(),
        }),
    );

    let mut manager = PluginManager::new(vec![Box::new(loader)]);
    manager.discover_all(Path::new("plugins")).await.unwrap();
    let errors = manager.load_and_enable_all(&factory).await;
    assert!(errors.is_empty());

    // Fire the event through the same EventBus
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

    assert!(handler_called.load(Ordering::SeqCst));

    manager.shutdown().await;
}
```

### Testing dependency order

The `PluginManager` resolves dependencies using topological sort. You can verify enable order with a shared counter:

```rust
let order = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

struct OrderPlugin {
    meta: PluginMetadata,
    order: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Plugin for OrderPlugin {
    fn metadata(&self) -> PluginMetadata { self.meta.clone() }
    fn on_enable<'a>(
        &'a self, _ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async {
            self.order.lock().unwrap().push(self.meta.id.clone());
            Ok(())
        })
    }
}

// Register child depending on parent
loader.register(
    PluginMetadata::new("child", "Child", "1.0")
        .depends_on("parent"), // [!code focus]
    move || Box::new(OrderPlugin {
        meta: PluginMetadata::new("child", "Child", "1.0")
            .depends_on("parent"),
        order: order_child.clone(),
    }),
);

loader.register(
    PluginMetadata::new("parent", "Parent", "1.0"),
    move || Box::new(OrderPlugin {
        meta: PluginMetadata::new("parent", "Parent", "1.0"),
        order: order_parent.clone(),
    }),
);

// After enable, parent appears before child
let enable_order = order.lock().unwrap();
assert_eq!(*enable_order, vec!["parent", "child"]);
```

### Testing cleanup on disable

When a plugin is disabled, the proxy automatically unsubscribes event listeners and unregisters commands through tracking wrappers (`TrackingEventBus`, `TrackingCommandManager`, `TrackingScheduler`). You can verify this by firing events after shutdown:

```rust
use infrarust_api::events::proxy::ProxyInitializeEvent;

#[tokio::test]
async fn test_listeners_removed_after_shutdown() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let event_bus = Arc::new(EventBusImpl::new());

    // ... build services with this event_bus, register plugin ...

    // Fire before shutdown — handler runs
    event_bus.fire(ProxyInitializeEvent).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // Shutdown removes all listeners
    manager.shutdown().await;

    // Fire again — handler does NOT run
    event_bus.fire(ProxyInitializeEvent).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
}
```

## The StaticPluginLoader

`StaticPluginLoader` is the registration mechanism for plugins compiled into the binary. It takes a `PluginMetadata` and a factory closure:

```rust
use infrarust_core::plugin::static_loader::StaticPluginLoader;

let loader = StaticPluginLoader::new();

loader.register(
    PluginMetadata::new("my_plugin", "My Plugin", "1.0.0"),
    || Box::new(MyPlugin::new()),
);
```

The factory closure is called each time the `PluginManager` loads the plugin. This means each test gets a fresh plugin instance.

::: warning
Registering two plugins with the same ID panics. Each plugin must have a unique `id` in its `PluginMetadata`.
:::

## Running tests

Plugin tests live alongside the code, either as `#[cfg(test)]` modules inside your plugin crate or as integration tests in `crates/infrarust-core/tests/`.

```bash
# Run all plugin-related tests
cargo test -p infrarust-core --test plugin_integration
cargo test -p infrarust-core --test plugin_manager
cargo test -p infrarust-core --test plugin_context

# Run tests in a specific plugin crate
cargo test -p infrarust-plugin-hello
```

Add `infrarust-core` as a dev-dependency in your plugin crate if you need the real service implementations:

```toml
[dev-dependencies]
infrarust-core = { path = "../../crates/infrarust-core" }
infrarust-api = { path = "../../crates/infrarust-api" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
uuid = "1"
```

## Summary of test patterns

| What you're testing | Mock level | Key types |
|---|---|---|
| `on_enable` / `on_disable` logic | `MockPluginContext` with `unimplemented!` stubs | `Plugin`, `PluginContext` |
| Event subscription and dispatch | Real `EventBusImpl` + mock services | `EventBusExt::subscribe`, `EventBusImpl::fire` |
| Command registration | Real `CommandManagerImpl` | `CommandManager::register` |
| Dependency ordering | `MockPluginContextFactory` + `PluginManager` | `PluginMetadata::depends_on` |
| Cleanup after disable | Real `PluginContextFactoryImpl` with tracking wrappers | `PluginManager::shutdown` |
| Full lifecycle | `PluginServices` + `StaticPluginLoader` + `PluginManager` | All of the above |

::: tip
Use `Arc<AtomicBool>` and `Arc<AtomicUsize>` to track callback invocations across async boundaries. These are `Send + Sync` and avoid lock contention in concurrent tests.
:::
