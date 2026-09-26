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
| `MemoryBanProvider` | `BanProvider` | Seeded bans, every `LoginAttempt` it was asked about |
| `MockPermissionChecker` | `PermissionChecker` | Every node it was asked about |
| `MockConfigService` | `ConfigService` | Server configs, documents and sources you add; proxy documents written |
| `MockServerManager` | `ServerManager` | Server states, `start` and `stop` calls |
| `MockLoadBalancerService` | `LoadBalancerService` | Backend pools and status changes |
| `MockPluginRegistry` | `PluginRegistry` | The `PluginInfo` values you add |
| `RecordingLimboSession` | `LimboSession` | Messages, titles, action bars, `complete` calls |
| `Gate` | | A barrier for holding a mock call open |
| `console()`, `console_with(checker)`, `player_source(&player)`, `command_context(source, label, args)` | `CommandSource`, `CommandContext` | |

The proxy-side counterparts (`PluginServices::for_tests`, `MockPluginContext`, `TestPlugin`) live in `infrarust_core::test_support`; see [Test support in infrarust-core](#test-support-in-infrarust-core).

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
| `stalled()` | `disconnect` never completes, to test code that must not wait on a player forever |
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

`add` replaces a player with the same ID, `remove(id)` takes one out, and `add_dyn` accepts any `Arc<dyn Player>`. `fake_online_count(n)` makes `online_count()` report `n` regardless of the players added, for code that reads the count without listing players.

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

`with_entry(entry)` and `insert(entry)` seed bans without going through `ban`, and `entries()` shows the store. `set_unavailable(true)` makes every call fail with `ServiceError::Unavailable`, to test what your plugin does when the ban store is down. `MockBanService::gated(gate)` holds every call at a [`Gate`](#gate) until the test opens it, and `MockBanService::panicking()` panics inside each call, for code that must survive a misbehaving service.

### MemoryBanProvider

A `BanProvider` for testing code that consumes a provider rather than the service: the proxy's ban manager, or a plugin that wraps another provider. `MemoryBanProvider::with(entries)` seeds it, `check` answers with the first matching entry and records the attempt in `attempts()`, `ban` stores entries with `mem1`, `mem2`, ... as ids, and `entries()` shows the store.

```rust
use infrarust_api::test_util::MemoryBanProvider;

let provider = MemoryBanProvider::with([
    BanEntry::new("seed", BanTarget::Username("Griefer".into()), BanSource::Console).reason("grief"),
])
.kick_message_with(|entry| Component::text(format!("Banned: {}", entry.reason.as_deref().unwrap_or(""))))
.ranges(true);
```

| Builder | Effect |
|---------|--------|
| `kick_message(component)`, `kick_message_with(fn)` | The `BanVerdict` message, fixed or built from the entry |
| `ranges(bool)` | What `features().ip_ranges` reports |
| `stuck()` | `check` never completes, to test ban timeouts |

### Gate

`Gate::new()` returns an `Arc<Gate>` that gated mocks await inside their calls. `gate.entered().await` resolves once a call is waiting at the gate, and `gate.open()` lets every waiting and future call through. A test that needs to observe a plugin while a service call is in flight starts the call, awaits `entered()`, makes its assertions, then opens the gate.

```rust
use infrarust_api::test_util::{Gate, MockBanService};

let gate = Gate::new();
let bans = Arc::new(MockBanService::gated(Arc::clone(&gate)));
let pending = tokio::spawn(check_login(Arc::clone(&bans), attempt));
gate.entered().await;
assert!(!pending.is_finished());
gate.open();
pending.await.unwrap();
```

### MockConfigService

A `ConfigService` that answers from the servers, documents and sources you add. `MockConfigService::server(id)` and `server_at(id, address)` build a `ServerConfig`, `server_document(id)` a TOML document and `file_source(id)` a `ServerSource`, so a test can populate it in one expression:

```rust
use infrarust_api::test_util::MockConfigService;

let config = MockConfigService::new()
    .with_server(MockConfigService::server("lobby"))
    .with_server_document("lobby", "name = \"lobby\"\n")
    .with_value("bind", "0.0.0.0:25565")
    .with_proxy_document("[proxy]\nbind = \"0.0.0.0:25565\"\n")
    .accepting_writes();
```

| Builder | Effect |
|---------|--------|
| `with_server(config)`, `with_servers(n)` | Add one server, or `n` generated ones |
| `with_server_document(id, toml)`, `with_source(source)` | What `get_server_document` and `list_server_sources` return |
| `with_value(key, value)` | What `get_value(key)` returns |
| `with_proxy_document(toml)` | The proxy document, also the starting point for writes |
| `redacting(fn)`, `effective_with(fn)` | Transform the document for `get_proxy_config_document` and `get_effective_proxy_config_document` |
| `accepting_writes()`, `merging_writes(fn)` | Accept `write_proxy_config_document` as is, or through a merge function; the default refuses with `ConfigWriteError::PermissionDenied` |

`written()` lists every document written and `stored_proxy_document()` the current one.

### MockServerManager, MockLoadBalancerService, MockPluginRegistry

- `MockServerManager::new().with_server("lobby", ServerState::Online)` answers `get_state`; `set_state` changes it while the plugin runs, and `started()` and `stopped()` list the servers the plugin asked to start or stop.
- `MockLoadBalancerService::new().with_server("lobby", "least_conn", addresses)` gives a server a pool of healthy backends; `set_status(&server, status)` changes one backend, and `set_drained` and `reset_backend` from the trait work on the pool.
- `MockPluginRegistry::new().with_plugin(info)` answers `list_plugin_info` and `plugin_info` from the `PluginInfo` values you add.

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

A `LimboSession` for testing a `LimboHandler` without the proxy. The handles it mints through `handle()` record into the same session.

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

## Test support in infrarust-core

The mocks above cover what a plugin receives. Tests that go through the proxy's own implementations (the `PluginManager`, a real event bus, the context factory) use `infrarust_core::test_support`, behind the `test-support` feature of `infrarust-core`, which turns on `infrarust-api/test-util` as well:

```toml
[dev-dependencies]
infrarust-core = { version = "2.0.0-beta.3", features = ["test-support"] }
```

| Item | What it gives you |
|------|-------------------|
| `PluginServices::for_tests()` | A complete `PluginServices`: real `EventBusImpl`, `CommandManagerImpl`, `SchedulerImpl`, `PluginRegistryImpl` and filter registries, the `test_util` mocks for the player registry, ban service, config service and load balancer, `NoopServerManager`, a fresh shutdown token and a default `ProxyInfo` |
| `PluginServices::for_tests_with(event_bus)` | The same around an event bus you keep a handle on |
| `MockPluginContext` | A `PluginContext` whose service accessors panic with `unimplemented!("mock")`, for `on_enable` tests that must not reach any service |
| `MockPluginContextFactory` | A `PluginContextFactory` that hands out `MockPluginContext`s |
| `TestPlugin` | A recording `Plugin` for lifecycle tests |
| `join_game_frame(version)` | The `JoinGame` packet the limbo sends, encoded for a protocol version |

### MockPluginContext

`MockPluginContext::new(plugin_id)` answers `plugin_id()`, `data_dir()` (`plugins/<id>`), `capabilities()`, `proxy_shutdown()`, and returns `None` from `codec_filters()` and `transport_filters()`; `register_config_provider` is a no-op. Every service accessor panics, so a test that reaches one fails loudly. The capabilities default to `CapabilitySet::native_trusted()`; `with_capabilities(CapabilitySet::baseline())` gives the default grant set instead.

```rust
use infrarust_api::permissions::CapabilitySet;
use infrarust_core::test_support::MockPluginContext;

let ctx = MockPluginContext::new("my_plugin").with_capabilities(CapabilitySet::baseline());
plugin.on_enable(&ctx).await.unwrap();
```

For a context with working services, build the real one: `PluginContextFactoryImpl::new(PluginServices::for_tests(), HashMap::new()).context("my_plugin")` returns an `Arc<PluginContextImpl>` whose event bus, commands and scheduler work without a proxy. The second argument maps plugin ids to their configured `PluginPermissions`; an empty map gives every plugin the baseline grant set.

### TestPlugin

A `Plugin` that records what the manager did to it. `TestPlugin::new(id)` builds one with version `1.0.0`; `version(v)` and `depends_on(id)` change its metadata, `fail_on_enable()` makes `on_enable` return `PluginError::InitFailed`, `on_enable(hook)` runs a closure with the context, and `register(&loader)` adds it to a `StaticPluginLoader`. After the run, `is_enabled()`, `enable_calls()` and `disable_calls()` report the lifecycle. For ordering, give several plugins the same counter with `sharing_order(&counter)`: `enable_order()` and `disable_order()` then return the position of each call.

## Unit testing a plugin

A unit test creates your plugin, calls `on_enable` with a mock context, and checks the result. Use `Arc<AtomicBool>` flags to verify that callbacks fire.

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use infrarust_core::test_support::MockPluginContext;

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

    let ctx = MockPluginContext::new("my_plugin");

    plugin.on_enable(&ctx).await.unwrap();
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
    let ctx = MockPluginContext::new("fail");

    let result = plugin.on_enable(&ctx).await;
    assert!(matches!(result, Err(PluginError::InitFailed(_))));
}
```

## Integration testing with real services

For tests that need real event dispatch, command registration, or scheduling, use the actual `infrarust-core` implementations alongside your mocks.

### Building PluginServices

`PluginServices` holds every service the proxy passes to plugins. `PluginServices::for_tests()` fills all of them with the real in-memory implementations and the `test_util` mocks; keep a handle on the event bus with `for_tests_with` when the test fires events itself, and replace individual fields for the mocks a test wants to inspect:

```rust
use std::collections::HashMap;
use infrarust_api::test_util::MockBanService;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::PluginContextFactoryImpl;
use infrarust_core::plugin::manager::PluginServices;

let event_bus = Arc::new(EventBusImpl::new());
let bans = Arc::new(MockBanService::new());
let services = PluginServices {
    ban_service: Arc::clone(&bans) as _,
    ..PluginServices::for_tests_with(Arc::clone(&event_bus))
};

let factory = PluginContextFactoryImpl::new(services, HashMap::new());
```

`MockLoadBalancerService::new().with_server("lobby", "least_conn", addresses)` gives a server healthy backends that `set_drained` and `reset_backend` then change, and `NoopServerManager` (the default) is the built-in stub for proxies without managed servers. `proxy_shutdown` is a `CancellationToken` the proxy triggers at shutdown, and `proxy_info` carries static metadata; both have test defaults. The second argument to `PluginContextFactoryImpl::new` is a `HashMap<String, PluginPermissions>` mapping plugin ids to their configured capabilities; an empty map gives every plugin the baseline grant set.

### Testing event handling

Register a plugin, fire an event, and assert the handler was called:

```rust
use infrarust_api::event::bus::EventBusExt;
use infrarust_api::event::EventPriority;
use infrarust_api::events::lifecycle::PostLoginEvent;
use infrarust_api::test_util::MockPlayer;
use infrarust_core::plugin::manager::PluginManager;
use infrarust_core::plugin::static_loader::StaticPluginLoader;
use infrarust_core::test_support::TestPlugin;

#[tokio::test]
async fn test_plugin_receives_post_login() {
    let handler_called = Arc::new(AtomicBool::new(false));

    // ... build PluginServices and factory as shown above ...

    let loader = StaticPluginLoader::new();
    let flag = handler_called.clone();
    TestPlugin::new("event_test")
        .on_enable(move |ctx| {
            let flag = flag.clone();
            ctx.event_bus().subscribe(
                EventPriority::NORMAL,
                move |_event: &mut PostLoginEvent| {
                    flag.store(true, Ordering::SeqCst);
                },
            );
        })
        .register(&loader);

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

The `PluginManager` resolves dependencies using topological sort. Give the plugins one shared counter and read the position each `on_enable` took:

```rust
use std::sync::atomic::AtomicUsize;
use infrarust_core::test_support::TestPlugin;

let order = Arc::new(AtomicUsize::new(0));
let parent = TestPlugin::new("parent").sharing_order(&order);
let child = TestPlugin::new("child")
    .depends_on("parent") // [!code focus]
    .sharing_order(&order);

let loader = StaticPluginLoader::new();
child.register(&loader);
parent.register(&loader);

// ... discover and enable through the PluginManager ...

assert!(parent.enable_order() < child.enable_order());
assert!(child.disable_order() < parent.disable_order());
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
    let ctx = factory.context("reminders");
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

    ctx.cleanup();
    assert_eq!(ctx.tracked_tasks(), 0);

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
infrarust-core = { path = "../../crates/infrarust-core", features = ["test-support"] }
infrarust-api = { path = "../../crates/infrarust-api", features = ["test-util"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
uuid = "1"
```

## Summary of test patterns

| What you're testing | Mock level | Key types |
|---|---|---|
| Code that talks to players, bans or permissions | `infrarust_api::test_util` mocks | `MockPlayer`, `MockPlayerRegistry`, `MockBanService`, `MemoryBanProvider`, `MockPermissionChecker` |
| Code that reads configuration, server states or backend pools | `infrarust_api::test_util` mocks | `MockConfigService`, `MockServerManager`, `MockLoadBalancerService`, `MockPluginRegistry` |
| A service call that must be observed in flight | `Gate` | `MockBanService::gated`, `Gate::entered`, `Gate::open` |
| A limbo handler | `RecordingLimboSession` | `LimboHandler::on_player_enter`, `completions()` |
| `on_enable` / `on_disable` logic | `infrarust_core::test_support::MockPluginContext` | `Plugin`, `PluginContext` |
| Event subscription and dispatch | Real `EventBusImpl` + mock services | `EventBusExt::subscribe`, `EventBusImpl::fire` |
| Command registration and dispatch | Real `CommandManagerImpl` behind `PluginContextFactoryImpl` | `CommandManager::register`, `CommandManagerImpl::dispatch(CommandSource::console(Arc::new(AllPermissionsChecker)), "name args")` |
| Dependency ordering | `TestPlugin` + `PluginManager` | `TestPlugin::depends_on`, `sharing_order`, `enable_order` |
| Cleanup after disable | Real `PluginContextFactoryImpl` with tracking wrappers | `PluginManager::shutdown`, `PluginManager::disable_plugin` |
| Scheduled tasks | Real context, paused tokio clock | `#[tokio::test(start_paused = true)]`, `tracked_tasks()` |
| Full lifecycle | `PluginServices::for_tests` + `StaticPluginLoader` + `PluginManager` | All of the above |

::: tip
Use `Arc<AtomicBool>` and `Arc<AtomicUsize>` to track callback invocations across async boundaries. These are `Send + Sync` and avoid lock contention in concurrent tests.
:::
