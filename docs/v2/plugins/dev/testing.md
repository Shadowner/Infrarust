---
title: Testing Plugins
description: Write unit and integration tests for Infrarust plugins using mock services, the static loader, and the plugin manager.
outline: [2, 3]
---

# Testing Plugins

Infrarust's plugin system is built on trait objects, which makes testing straightforward. You can mock individual services, wire up a real event bus, or run a full plugin lifecycle through the `PluginManager`.

This page covers three levels of testing, from isolated unit tests to end-to-end integration tests.

## Mock services

The service traits a plugin receives (`Player`, `PlayerRegistry`, `BanService` and the rest) are sealed: only the proxy implements them in production. Tests still need stand-ins, so `infrarust-api` ships ready-made mocks behind its `test-util` feature. Turn it on for tests only:

```toml
[dependencies]
infrarust-api = "2.0.0-beta.3"

[dev-dependencies]
infrarust-api = { version = "2.0.0-beta.3", features = ["test-util"] }
```

Everything lives in `infrarust_api::test_util`:

| Mock | Stands in for | Records |
|------|---------------|---------|
| `MockPlayer` | `Player` | Messages, titles, action bars, packets, kicks, server switches, permission refreshes |
| `MockPlayerRegistry` | `PlayerRegistry` | The players you add |
| `MockBanService` | `BanService` | Bans in memory, every `check` it answered |
| `MockPermissionChecker` | `PermissionChecker` | Every node it was asked about |
| `RecordingLimboSession` | `LimboSession` | Messages, titles, action bars, `complete` calls |
| `console()`, `console_with(checker)`, `player_source(&player)`, `command_context(source, label, args)` | `CommandSource`, `CommandContext` | |

### MockPlayer

A connected, active player on protocol 1.21 at `127.0.0.1:25565`, with the UUID built from its ID and no permissions. Builder methods change what the plugin sees:

```rust
use infrarust_api::prelude::*;
use infrarust_api::test_util::MockPlayer;

let steve = MockPlayer::new(1, "Steve")
    .on_server("lobby")
    .online_mode(true)
    .with_permission("hub.use")
    .into_arc();

my_plugin.greet(steve.as_ref());

assert_eq!(steve.sent_text(), "Welcome to the hub");
assert!(steve.kicks().is_empty());
```

| Builder | Effect |
|---------|--------|
| `with_profile(GameProfile)` | Replace the UUID, username and properties |
| `with_protocol_version(v)`, `with_remote_addr(addr)` | Client version and address |
| `online_mode(bool)` | What `is_online_mode()` returns |
| `on_server(id)` | What `current_server()` returns |
| `passive()` | `is_active()` returns `false` and the `send_*` methods fail with `PlayerError::NotActive`, like a passthrough player |
| `with_permission(node)`, `without_permission(node)`, `with_all_permissions()` | Grant or deny nodes. Wildcards such as `hub.*` work as in `PermissionMap` |
| `with_permissions(Arc<MockPermissionChecker>)` | Share one checker between players |
| `into_arc()` | `Arc::new(self)` |

After `disconnect`, the player reports `is_connected() == false`, the reason is in `kicks()`, and further `send_*` calls fail with `PlayerError::Disconnected`. `switch_server` records the target in `switches()` and moves `current_server()` to it. `permissions()` returns the player's checker, so a test can change a node while the plugin runs.

### MockPlayerRegistry

Answers lookups from the players you add, the way the proxy does: usernames match case-insensitively, `get_players_on_server` and `online_count_on` use each player's `current_server()`.

```rust
use infrarust_api::test_util::{MockPlayer, MockPlayerRegistry};

let steve = MockPlayer::new(1, "Steve").on_server("lobby").into_arc();
let registry = Arc::new(MockPlayerRegistry::new().with(steve.clone()));
registry.add(MockPlayer::new(2, "Alex").into_arc());

let plugin = AfkKicker::new(registry.clone() as Arc<dyn PlayerRegistry>);
```

`add` replaces a player with the same ID, `remove(id)` takes one out, and `add_dyn` accepts any `Arc<dyn Player>`.

### MockBanService

An in-memory ban store. `ban` stores an entry with an increasing ID (the source defaults to `BanSource::System`), `check` returns a `BanVerdict` for the first unexpired entry that matches the attempt, IP ranges included, and `list` pages with the entry ID as cursor.

```rust
use infrarust_api::test_util::MockBanService;

let bans = Arc::new(MockBanService::new());
bans.ban(BanRequest::new(BanTarget::Username("griefer".into()))).await?;

let verdict = bans
    .check(&LoginAttempt::pre_auth("10.0.0.1".parse()?, "Griefer"))
    .await?;
assert!(verdict.is_some());
assert_eq!(bans.checks().len(), 1);
```

`with_entry(entry)` and `insert(entry)` seed bans without going through `ban`, and `entries()` shows the store. `set_unavailable(true)` makes every call fail with `ServiceError::Unavailable`, to test what your plugin does when the ban store is down.

### Permissions and command sources

`MockPermissionChecker` wraps a `PermissionMap` and records the nodes it was asked about:

```rust
use infrarust_api::test_util::{MockPermissionChecker, command_context, console, console_with, player_source};

let checker = Arc::new(MockPermissionChecker::new().grant("auth.admin.*").deny("auth.admin.purge"));
let ctx = command_context(console_with(checker.clone()), "purge", "Steve");
PurgeCommand.execute(ctx).await;
assert_eq!(checker.checked(), ["auth.admin.purge"]);

let steve = MockPlayer::new(1, "Steve").into_arc();
PurgeCommand.execute(command_context(player_source(&steve), "purge", "Alex")).await;
assert!(steve.sent_text().contains("permission"));
```

`MockPermissionChecker::allow_all()` grants everything, and `console()` is a console that holds every permission. Replies to the console go to the log, so use a player source when the test needs to read the reply.

### RecordingLimboSession

A `LimboSession` for testing a `LimboHandler` without the proxy. It is also reachable as `infrarust_api::limbo::test_util::RecordingLimboSession`.

```rust
use infrarust_api::test_util::RecordingLimboSession;

let session = RecordingLimboSession::new(
    PlayerId::new(1),
    GameProfile { uuid: uuid::Uuid::nil(), username: "Steve".into(), properties: vec![] },
    LimboEntryContext::InitialConnection { target_server: ServerId::new("lobby") },
);

assert!(matches!(handler.on_player_enter(session.as_ref()).await, HandlerResult::Hold));
handler.on_command(session.as_ref(), "login", &["hunter2"]).await;
assert!(matches!(session.completions()[..], [HandlerResult::Accept]));
```

The auth plugin's tests (`plugins/infrarust-plugin-auth/src/test_support.rs`) are built on these mocks.

### Hand-written mocks

The services without a mock in `test_util` (`ConfigService`, `ServerManager`, `LoadBalancerService`, `PluginRegistry`) are sealed through a public `private::Sealed` marker, so a test mock implements both the service trait and `Sealed`. Infrarust's own test suite has no-op versions in `crates/infrarust-core/tests/mock_services/mod.rs`.

### MockConfigService

Returns `None` for all config lookups and refuses writes:

```rust
use infrarust_api::services::config_service::{
    ConfigWriteError, ServerConfig, ServerSource,
};
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
    fn get_server_document(&self, _server: &ServerId) -> Option<String> {
        None
    }
    fn list_server_sources(&self) -> Vec<ServerSource> { vec![] }
    fn get_proxy_config_document(&self) -> String { String::new() }
    fn get_effective_proxy_config_document(&self) -> String {
        String::new()
    }
    fn write_proxy_config_document(
        &self, _toml: &str,
    ) -> Result<(), ConfigWriteError> {
        Err(ConfigWriteError::PermissionDenied)
    }
    fn get_value(&self, _key: &str) -> Option<String> { None }
}
```

### MockPluginContext

When you need to test `on_enable` in isolation without a real `PluginManager`, you can implement `PluginContext` directly. The trait is large, so stub the methods you don't need with `unimplemented!("mock")` and your test panics if the plugin reaches for something unexpected. Every method on the trait must be present or the impl will not compile. This mirrors the mock in `crates/infrarust-core/src/plugin/static_loader.rs`.

```rust
use std::sync::Arc;
use infrarust_api::permissions::CapabilitySet;
use infrarust_api::plugin::PluginContext;

struct MockPluginContext {
    plugin_id: String,
    capabilities: CapabilitySet,
}

impl infrarust_api::plugin::private::Sealed for MockPluginContext {}

impl PluginContext for MockPluginContext {
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn plugin_id(&self) -> &str { &self.plugin_id }
    fn data_dir(&self) -> std::path::PathBuf {
        std::path::PathBuf::from("plugins").join(&self.plugin_id)
    }
    fn capabilities(&self) -> &CapabilitySet { &self.capabilities }

    fn event_bus(&self) -> &dyn infrarust_api::event::bus::EventBus {
        unimplemented!("mock")
    }
    fn event_bus_handle(
        &self,
    ) -> Arc<dyn infrarust_api::event::bus::EventBus> {
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
    fn register_ban_provider(
        &self,
        _provider: Arc<dyn infrarust_api::services::ban_service::BanProvider>,
    ) -> Result<(), infrarust_api::services::ban_service::BanProviderRejected> {
        unimplemented!("mock")
    }
    fn register_permission_provider(
        &self,
        _provider: Arc<dyn infrarust_api::permissions::PermissionProvider>,
    ) -> Result<(), infrarust_api::permissions::PermissionProviderRejected> {
        unimplemented!("mock")
    }
    fn register_permission_node(
        &self, _node: infrarust_api::permissions::PermissionNode,
    ) -> Result<(), infrarust_api::permissions::PermissionNodeError> {
        unimplemented!("mock")
    }
    fn permission_nodes(
        &self,
    ) -> Vec<infrarust_api::permissions::PermissionNodeInfo> {
        Vec::new()
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
    fn load_balancer_service(
        &self,
    ) -> &dyn infrarust_api::services::load_balancer::LoadBalancerService {
        unimplemented!("mock")
    }
    fn load_balancer_service_handle(
        &self,
    ) -> Arc<dyn infrarust_api::services::load_balancer::LoadBalancerService> {
        unimplemented!("mock")
    }
    fn plugin_registry(
        &self,
    ) -> &dyn infrarust_api::services::plugin_registry::PluginRegistry {
        unimplemented!("mock")
    }
    fn plugin_registry_handle(
        &self,
    ) -> Arc<dyn infrarust_api::services::plugin_registry::PluginRegistry> {
        unimplemented!("mock")
    }
    fn command_manager(
        &self,
    ) -> &dyn infrarust_api::command::CommandManager {
        unimplemented!("mock")
    }
    fn command_manager_handle(
        &self,
    ) -> Arc<dyn infrarust_api::command::CommandManager> {
        unimplemented!("mock")
    }
    fn scheduler(
        &self,
    ) -> &dyn infrarust_api::services::scheduler::Scheduler {
        unimplemented!("mock")
    }
    fn scheduler_handle(
        &self,
    ) -> Arc<dyn infrarust_api::services::scheduler::Scheduler> {
        unimplemented!("mock")
    }
    fn services(
        &self,
    ) -> &dyn infrarust_api::services::ServiceRegistry {
        unimplemented!("mock")
    }
    fn services_handle(
        &self,
    ) -> Arc<dyn infrarust_api::services::ServiceRegistry> {
        unimplemented!("mock")
    }
    fn register_limbo_handler(
        &self, _handler: Box<dyn infrarust_api::limbo::LimboHandler>,
    ) -> Result<
        infrarust_api::limbo::LimboHandlerRegistration,
        infrarust_api::limbo::LimboHandlerError,
    > {
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
    fn proxy_shutdown(&self) -> tokio_util::sync::CancellationToken {
        tokio_util::sync::CancellationToken::new()
    }
    fn proxy_info(&self) -> &infrarust_api::services::proxy_info::ProxyInfo {
        unimplemented!("mock")
    }
}
```

The `capabilities` field controls what `codec_filters()` and `transport_filters()` would return on a real context, and it is what a plugin reads through `ctx.capabilities()`. For most tests `CapabilitySet::native_trusted()` grants everything; `CapabilitySet::baseline()` gives the default grant set instead.

You can also wrap this in a factory so the `PluginManager` can create per-plugin contexts:

```rust
use infrarust_api::permissions::CapabilitySet;
use infrarust_core::plugin::PluginContextFactory;

struct MockPluginContextFactory;

impl PluginContextFactory for MockPluginContextFactory {
    fn create_context(
        &self, plugin_id: &str,
    ) -> Arc<dyn PluginContext> {
        Arc::new(MockPluginContext {
            plugin_id: plugin_id.to_string(),
            capabilities: CapabilitySet::native_trusted(),
        })
    }
}
```

`PluginContextFactory` is defined in `infrarust-api` (`infrarust_api::loader::PluginContextFactory`) and re-exported from `infrarust_core::plugin`. The trait also has a `forget_context(&self, plugin_id: &str)` method with a no-op default, so the impl above is complete.

## Unit testing a plugin

A unit test creates your plugin, calls `on_enable` with a mock context, and checks the result. Use `Arc<AtomicBool>` flags to verify that callbacks fire.

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::permissions::CapabilitySet;
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
        capabilities: CapabilitySet::native_trusted(),
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
        capabilities: CapabilitySet::native_trusted(),
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
use tokio_util::sync::CancellationToken;
use infrarust_api::services::proxy_info::ProxyInfo;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::{PluginContextFactoryImpl, PluginRegistryImpl};
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;

let event_bus = Arc::new(EventBusImpl::new());

let services = PluginServices {
    event_bus: Arc::clone(&event_bus),
    player_registry: Arc::new(MockPlayerRegistry::new()),
    server_manager: Arc::new(NoopServerManager),
    ban_service: Arc::new(MockBanService::new()),
    command_manager: Arc::new(CommandManagerImpl::new()),
    scheduler: Arc::new(SchedulerImpl::new()),
    config_service: Arc::new(MockConfigService),
    load_balancer_service: Arc::new(MockLoadBalancerService),
    plugin_registry: Arc::new(PluginRegistryImpl::new()),
    codec_filter_registry: Arc::new(
        infrarust_core::filter::codec_registry::CodecFilterRegistryImpl::new(),
    ),
    transport_filter_registry: Arc::new(
        infrarust_core::filter::transport_registry::TransportFilterRegistryImpl::new(),
    ),
    domain_router: Arc::new(
        infrarust_core::routing::DomainRouter::new(),
    ),
    proxy_shutdown: CancellationToken::new(),
    proxy_info: ProxyInfo::default(),
    plugins_dir: PathBuf::from("plugins"),
};

let factory = PluginContextFactoryImpl::new(
    services, HashMap::new(),
);
```

`PluginServices` has every field the proxy fills in, so a test has to supply all of them. The `MockPlayerRegistry` and `MockBanService` from `infrarust_api::test_util` fit the `player_registry` and `ban_service` fields. `EventBusImpl`, `CommandManagerImpl`, `SchedulerImpl`, and `PluginRegistryImpl` are real implementations that work without any proxy infrastructure. `NoopServerManager` is a built-in stub for proxies without managed servers. `proxy_shutdown` is a `CancellationToken` the proxy triggers at shutdown, and `proxy_info` carries static metadata; both have sensible test defaults. The second argument to `PluginContextFactoryImpl::new` is a `HashMap<String, PluginPermissions>` mapping plugin IDs to their configured capabilities; an empty map gives every plugin the baseline grant set.

### Testing event handling

Register a plugin, fire an event, and assert the handler was called:

```rust
use infrarust_api::event::bus::EventBusExt;
use infrarust_api::event::EventPriority;
use infrarust_api::events::lifecycle::PostLoginEvent;
use infrarust_api::test_util::MockPlayer;
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
    let event = PostLoginEvent::new(MockPlayer::new(1, "TestPlayer").into_arc());
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

    // Fire before shutdown: handler runs
    event_bus.fire(ProxyInitializeEvent).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // Shutdown removes all listeners
    manager.shutdown().await;

    // Fire again: handler does NOT run
    event_bus.fire(ProxyInitializeEvent).await;
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
}
```

## Testing scheduled work

Scheduler tests should not sleep for real. Start the test with tokio's clock paused: time only moves when every task is waiting, and it jumps straight to the next timer, so a test that waits an hour takes microseconds and never flakes. It needs tokio's `test-util` feature in your dev-dependencies.

```rust
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

#[tokio::test(start_paused = true)]
async fn the_reminder_repeats_until_the_plugin_is_disabled() {
    let factory = factory();
    let ctx = factory.create_context("reminders");
    let runs = Arc::new(AtomicU32::new(0));
    let counted = runs.clone();
    ctx.scheduler().repeat(
        Duration::from_secs(60),
        None,
        Box::new(move || {
            let counted = counted.clone();
            Box::pin(async move {
                counted.fetch_add(1, Ordering::SeqCst);
            })
        }),
    );

    tokio::time::sleep(Duration::from_secs(150)).await;
    assert_eq!(runs.load(Ordering::SeqCst), 2);

    let real = ctx.as_any().downcast_ref::<PluginContextImpl>().unwrap();
    real.cleanup();
    assert_eq!(real.tracked_tasks(), 0);

    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}
```

`factory()` builds a `PluginContextFactoryImpl` as in [Building PluginServices](#building-pluginservices). `cleanup()` is what the proxy runs when it disables the plugin, and `tracked_tasks()` counts the plugin's tasks that are still scheduled; finished tasks are not counted.

Keep a margin between a timer and the moment you check it: the clock has millisecond resolution, so a check at exactly the deadline can land on either side.

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

A plugin only needs `infrarust-api` to build, but the test patterns above also reach into `infrarust-core` for the real service implementations (`EventBusImpl`, `PluginManager`, `StaticPluginLoader`, and friends). Add it as a dev-dependency. The plugin crates that ship in this repo are workspace members and write `infrarust-core = { workspace = true }`; an out-of-tree plugin uses a path or version instead:

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
| Code that talks to players, bans or permissions | `infrarust_api::test_util` mocks | `MockPlayer`, `MockPlayerRegistry`, `MockBanService`, `MockPermissionChecker` |
| A limbo handler | `RecordingLimboSession` | `LimboHandler::on_player_enter`, `completions()` |
| `on_enable` / `on_disable` logic | `MockPluginContext` with `unimplemented!` stubs | `Plugin`, `PluginContext` |
| Event subscription and dispatch | Real `EventBusImpl` + mock services | `EventBusExt::subscribe`, `EventBusImpl::fire` |
| Command registration and dispatch | Real `CommandManagerImpl` behind `PluginContextFactoryImpl` | `CommandManager::register`, `CommandManagerImpl::dispatch(CommandSource::console(Arc::new(AllPermissionsChecker)), "name args")` |
| Dependency ordering | `PluginContextFactoryImpl` + `PluginManager` | `PluginMetadata::depends_on` |
| Cleanup after disable | Real `PluginContextFactoryImpl` with tracking wrappers | `PluginManager::shutdown`, `PluginManager::disable_plugin` |
| Scheduled tasks | Real context, paused tokio clock | `#[tokio::test(start_paused = true)]`, `tracked_tasks()` |
| Full lifecycle | `PluginServices` + `StaticPluginLoader` + `PluginManager` | All of the above |

::: tip
Use `Arc<AtomicBool>` and `Arc<AtomicUsize>` to track callback invocations across async boundaries. These are `Send + Sync` and avoid lock contention in concurrent tests.
:::
