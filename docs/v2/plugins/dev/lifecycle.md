---
title: Plugin Lifecycle
description: How Infrarust plugins are discovered, loaded, enabled, and shut down. Covers the Plugin trait, PluginMetadata, dependency resolution, and automatic resource cleanup.
outline: [2, 3]
---

# Plugin Lifecycle

Every plugin goes through a fixed sequence of stages: discovery, dependency resolution, loading, enabling, active operation, and shutdown. The `PluginManager` orchestrates this sequence and handles errors at each step.

## Lifecycle stages

```
discover_all()               load_and_enable_all()                shutdown()
     │                              │                                │
     ▼                              ▼                                ▼
┌──────────┐  ┌───────────┐  ┌───────────┐  ┌─────────┐  ┌──────────┐  ┌──────────┐
│ Discover │─▶│ Resolve   │─▶│  Loading  │─▶│ Enabled │─▶│ Disabled │─▶│ Unloaded │
│          │  │ deps      │  │           │  │         │  │          │  │          │
└──────────┘  └───────────┘  └─────┬─────┘  └─────────┘  └──────────┘  └──────────┘
                                   │
                                   ▼
                              ┌─────────┐
                              │  Error  │
                              └─────────┘
```

**Discover.** Each `PluginLoader` scans the plugin directory and returns `PluginMetadata` for every plugin it can load. The manager rejects duplicate IDs across loaders.

**Resolve deps.** The manager runs a topological sort (Kahn's algorithm) on the collected metadata. This determines load order so that dependencies are enabled before the plugins that need them.

**Loading.** Before any plugin loads, the manager calls `loader.on_load()` once per loader so loaders that host a runtime (the WASM loader, for example) can initialize it. Then for each plugin in the resolved order it calls `loader.load()` and sets the plugin's state to `Loading`. If a loader's `on_load()` fails, all of that loader's plugins are skipped and marked `Error`.

**Enabled.** The manager calls `plugin.on_enable(ctx)`. If it succeeds, state moves to `Enabled`. If it fails, state moves to `Error` and the context is cleaned up immediately.

**Disabled.** During shutdown, the manager iterates plugins in reverse order. It calls `on_disable()`, then runs automatic cleanup regardless of whether `on_disable` succeeded.

**Unloaded.** After all plugins are disabled, the manager calls `loader.unload()` for each plugin to release loader-level resources, then `loader.on_shutdown()` once per loader (only for loaders whose `on_load()` succeeded) to tear down the runtime.

## PluginState

The `PluginState` enum tracks where a plugin is in the lifecycle:

```rust
pub enum PluginState {
    Loading,         // on_enable() in progress
    Enabled,         // Active and operational
    Disabled,        // After shutdown
    Error(String),   // Failed during init, message explains why
}
```

Query a plugin's state programmatically via `PluginManager::plugin_state()`, which returns `Option<&PluginState>`. The in-game `plugins` subcommand lists loaded plugins, and `plugin <id>` lists or runs a plugin's commands, but neither exposes the raw lifecycle state.

## The Plugin trait

The `Plugin` trait is the entry point for all plugins. It's defined in `crates/infrarust-api/src/plugin.rs`:

```rust
pub trait Plugin: Send + Sync {
    fn metadata(&self) -> PluginMetadata;

    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>>;

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async { Ok(()) })  // default: no-op
    }
}
```

`metadata()` returns the plugin's identity and dependency declarations. Called multiple times throughout the lifecycle.

`on_enable()` receives a `PluginContext` for registering event listeners, commands, limbo handlers, config providers, and filters. This is the only place you should register resources, because the context tracks everything for automatic cleanup.

`on_disable()` is optional. Override it only if your plugin holds external resources (database connections, open files, network sockets) that need explicit teardown. Event listeners, commands, and scheduled tasks are cleaned up automatically.

::: warning
`on_enable` and `on_disable` return `BoxFuture` because the trait uses manual async dispatch. Wrap your implementation in `Box::pin(async move { ... })`.
:::

## PluginMetadata

`PluginMetadata` identifies your plugin and declares its dependencies:

```rust
pub struct PluginMetadata {
    pub id: String,                        // Unique snake_case identifier
    pub name: String,                      // Human-readable name
    pub version: String,                   // Semver version string
    pub authors: Vec<String>,              // Author list
    pub description: Option<String>,       // Optional description
    pub dependencies: Vec<PluginDependency>,
}
```

Build metadata using the constructor and builder methods:

```rust
PluginMetadata::new("my_plugin", "My Plugin", "1.0.0")
    .author("Alice")
    .author("Bob")
    .description("Does something useful")
    .depends_on("core_plugin")             // required dependency // [!code focus]
    .optional_dependency("extra_plugin")   // optional dependency // [!code focus]
```

The `id` field must be unique across all loaded plugins. The manager rejects duplicates during discovery.

## Dependencies

Each dependency is a `PluginDependency` with two fields:

```rust
pub struct PluginDependency {
    pub id: String,    // ID of the required plugin
    pub optional: bool, // true = plugin works without it
}
```

Use `.depends_on("plugin_id")` for required dependencies and `.optional_dependency("plugin_id")` for optional ones.

### Resolution rules

The dependency resolver in `crates/infrarust-core/src/plugin/dependency.rs` applies these rules:

1. If a required dependency is missing, the resolver returns an error and no plugins load.
2. If an optional dependency is missing, it's skipped. The declaring plugin still loads.
3. If an optional dependency is present, it still affects load order. The dependency loads first.
4. Circular dependencies (A depends on B, B depends on A) are detected and rejected.

The resolver uses Kahn's algorithm for topological sorting. Plugins with no dependencies load first, then plugins whose dependencies are satisfied, and so on.

```rust
// Example: three plugins with dependencies
//   auth depends on database
//   motd has no dependencies
//
// Resolved order: [motd, database, auth] or [database, motd, auth]
// database always loads before auth
```

::: danger
A missing required dependency prevents all plugins from loading, not just the one that declared the dependency. Fix missing dependencies before starting the proxy.
:::

## Enable flow in detail

When `load_and_enable_all()` runs, it first calls `on_load()` on every loader and records which ones succeeded. Then it processes each plugin in the resolved order:

1. Finds the correct loader for the plugin (based on discovery mapping). If that loader's `on_load()` failed, the plugin is marked `Error` and skipped.
2. Calls `loader.load(plugin_id, context_factory)` to instantiate the plugin.
3. Creates a per-plugin `PluginContext` via the context factory.
4. Sets state to `Loading`.
5. Calls `plugin.on_enable(ctx)`.
6. On success: state becomes `Enabled`, the plugin is stored for later shutdown.
7. On failure: state becomes `Error(message)`, the context is immediately cleaned up, and the error is collected.

Errors during loading or enabling don't stop other plugins. The manager collects all errors and continues with the next plugin in the load order.

## Shutdown flow

`shutdown()` disables plugins in reverse load order (last enabled, first disabled):

1. Skips any plugin not in the `Enabled` state.
2. Sets state to `Disabled`.
3. Calls `plugin.on_disable()`. Errors are logged but don't stop the shutdown.
4. Runs `cleanup()` on the plugin's context. This happens even if `on_disable` failed.
5. After all plugins are disabled, calls `loader.unload()` for each plugin.
6. Calls `loader.on_shutdown()` on each loader that booted successfully, in reverse order. Shutdown is idempotent, so calling it twice runs `on_shutdown()` only once.

## Automatic resource cleanup

Each plugin gets its own `PluginContext` backed by a `PluginContextImpl` that wraps proxy services with tracking decorators. When you register a listener, command, or scheduled task through the context, the registration is recorded.

During cleanup (on disable or on enable failure), the context automatically:

- Unsubscribes all event listeners registered through `ctx.event_bus()`
- Unregisters all commands registered through `ctx.command_manager()`
- Cancels all scheduled tasks registered through `ctx.scheduler()`
- Cancels config provider watch tokens
- Removes active provider route entries from the domain router

This means you don't need to manually unsubscribe listeners or cancel tasks in `on_disable`. The proxy handles it.

```rust
// During on_enable: just register, don't store the handle
ctx.event_bus().subscribe::<PostLoginEvent, _>(
    EventPriority::NORMAL,
    |event| {
        tracing::info!("{} joined", event.profile.username);
    },
);

// During shutdown: the proxy unsubscribes this automatically
```

::: tip
Only override `on_disable` if you have resources the proxy can't track: database connections, open file handles, background threads you spawned yourself, etc.
:::

## The PluginContext

`PluginContext` is a sealed trait. Only the proxy implements it. Plugins receive it as `&dyn PluginContext` during `on_enable`.

Some of the available services:

| Method | Returns | Purpose |
|--------|---------|---------|
| `event_bus()` | `&dyn EventBus` | Subscribe to events |
| `command_manager()` | `&dyn CommandManager` | Register console/player commands |
| `scheduler()` | `&dyn Scheduler` | Schedule delayed or repeating tasks |
| `player_registry()` | `&dyn PlayerRegistry` | Look up connected players |
| `server_manager()` | `&dyn ServerManager` | Query and manage backend servers |
| `ban_service()` | `&dyn BanService` | Ban/unban players |
| `config_service()` | `&dyn ConfigService` | Read proxy configuration |
| `plugin_registry()` | `&dyn PluginRegistry` | Inspect other loaded plugins |
| `register_limbo_handler()` | `()` | Register a limbo handler |
| `register_config_provider()` | `()` | Register a config provider |
| `codec_filters()` | `Option<&dyn CodecFilterRegistry>` | Register packet-level filters, `Some` only with the `CodecFilter` capability |
| `transport_filters()` | `Option<&dyn TransportFilterRegistry>` | Register TCP-level filters, `Some` only with the `TransportFilter` capability |
| `plugin_id()` | `&str` | This plugin's ID |
| `data_dir()` | `PathBuf` | This plugin's data directory |
| `proxy_shutdown()` | `CancellationToken` | A token that fires when the proxy shuts down |
| `proxy_info()` | `&ProxyInfo` | Proxy name and version |
| `capabilities()` | `&CapabilitySet` | Capabilities granted to this plugin |

The two filter registries return `None` unless the plugin holds the matching capability. Compiled-in native plugins are trusted and receive both; WASM plugins are gated by config.

Most accessors have an `_handle()` companion (`event_bus_handle()`, `player_registry_handle()`, and so on) that returns an `Arc` instead of a borrow. Use the `Arc` form when you need to capture the service in a closure or move it into a spawned async task.

## Plugin loaders

The `PluginLoader` trait abstracts how plugins are discovered and instantiated. Different loaders support different plugin formats.

```rust
pub trait PluginLoader: Send + Sync {
    fn name(&self) -> &str;

    fn discover<'a>(
        &'a self, plugin_dir: &'a Path,
    ) -> BoxFuture<'a, Result<Vec<PluginMetadata>, LoaderError>>;

    // Defaults to a no-op. Runs once, before any of this loader's plugins load.
    fn on_load<'a>(
        &'a self, context_factory: &'a dyn PluginContextFactory,
    ) -> BoxFuture<'a, Result<(), LoaderError>> { /* ... */ }

    fn load<'a>(
        &'a self, plugin_id: &'a str,
        context_factory: &'a dyn PluginContextFactory,
    ) -> BoxFuture<'a, Result<Box<dyn Plugin>, LoaderError>>;

    fn unload<'a>(
        &'a self, plugin_id: &'a str,
    ) -> BoxFuture<'a, Result<(), LoaderError>>;

    // Defaults to a no-op. Runs once, after all this loader's plugins unload.
    fn on_shutdown<'a>(&'a self) -> BoxFuture<'a, Result<(), LoaderError>> { /* ... */ }
}
```

`discover()` runs before the context factory exists, so a loader cannot rely on its own hosted runtime to enumerate plugins. `on_load()` is where a runtime gets initialized, and `on_shutdown()` is where it is torn down. Both have default no-op implementations, so simple loaders only implement `name`, `discover`, `load`, and `unload`.

Infrarust ships with a `StaticPluginLoader` that loads plugins compiled directly into the binary. Plugins are registered with a metadata struct and a factory closure. Its `discover()` ignores the plugin directory argument, and `register()` panics if you register two plugins with the same ID.

```rust
let loader = StaticPluginLoader::new();
loader.register(
    PluginMetadata::new("greet", "Greet Plugin", "0.1.0"),
    || Box::new(GreetPlugin),
);
```

The WASM loader is a second `PluginLoader` implementation that discovers and runs `wasm32-wasip2` components from the plugins directory. New formats slot in the same way, by adding another `PluginLoader`.

## Error handling

Errors during the lifecycle surface as `PluginError` or `LoaderError` depending on the source.

`LoaderError` covers loader-level failures:

| Variant | When |
|---------|------|
| `DirectoryNotAccessible` | Plugin directory can't be read |
| `PluginNotFound` | `load()` called for an unknown ID |
| `InvalidFormat` | Plugin file is corrupt or unreadable |
| `LoadFailed` | Plugin instantiation failed |
| `UnloadFailed` | Cleanup after disable failed |
| `DuplicateId` | Two loaders found plugins with the same ID |

`PluginError` has two variants:

| Variant | When |
|---------|------|
| `InitFailed(String)` | Dependency resolution failures (missing required dependency, circular dependency), loader discovery or `on_load` failures, and the message the manager records for a failed `load()`. |
| `Custom(String)` | Anything else. `String` and `&str` convert into this variant via `From`, so a plugin can return `Err("reason".into())` from `on_enable`. |

`discover_all()` returns a single `PluginError` on the first fatal problem (a duplicate ID, a loader discovery failure, or an unresolvable dependency graph), and no plugins load. `load_and_enable_all()` is more forgiving: it collects every error into a `Vec<PluginError>` and returns them. An error returned from a plugin's own `on_enable` is collected as-is, not re-wrapped. Each failed plugin is marked `PluginState::Error` while the remaining plugins continue loading.

During `shutdown`, errors from `on_disable` and `unload` are logged but don't interrupt the process. Every plugin gets its cleanup pass regardless of errors in other plugins.
