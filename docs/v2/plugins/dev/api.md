---
title: Plugin API Reference
description: Complete reference for PluginContext, services, and types available to Infrarust plugins.
outline: [2, 3]
---

# Plugin API Reference

Every plugin receives a `PluginContext` during `on_enable`. This is your gateway to the proxy's services: player registry, scheduler, server manager, ban service, config service, event bus, and command manager.

All services are trait objects. The proxy is the sole implementor. You access them through the context and, where needed, capture `Arc` handles for use inside closures.

```rust
use infrarust_api::prelude::*;

fn on_enable<'a>(&'a self, ctx: &'a dyn PluginContext) -> BoxFuture<'a, Result<(), PluginError>> {
    Box::pin(async move {
        let players = ctx.player_registry();
        let scheduler = ctx.scheduler();
        let servers = ctx.server_manager();
        // ...
        Ok(())
    })
}
```

## PluginContext

The `PluginContext` trait provides access to every service and registration method. It is sealed, so you cannot implement it yourself.

| Method | Returns | Purpose |
|--------|---------|---------|
| `event_bus()` | `&dyn EventBus` | Subscribe to proxy events |
| `event_bus_handle()` | `Arc<dyn EventBus>` | Cloneable handle for closures |
| `player_registry()` | `&dyn PlayerRegistry` | Look up connected players |
| `player_registry_handle()` | `Arc<dyn PlayerRegistry>` | Cloneable handle for closures |
| `server_manager()` | `&dyn ServerManager` | Query and control backend servers |
| `server_manager_handle()` | `Arc<dyn ServerManager>` | Cloneable handle for closures |
| `ban_service()` | `&dyn BanService` | Ban and unban players |
| `ban_service_handle()` | `Arc<dyn BanService>` | Cloneable handle for closures |
| `config_service()` | `&dyn ConfigService` | Read proxy and server configuration |
| `config_service_handle()` | `Arc<dyn ConfigService>` | Cloneable handle for closures |
| `command_manager()` | `&dyn CommandManager` | Register and unregister commands |
| `scheduler()` | `&dyn Scheduler` | Schedule delayed and recurring tasks |
| `codec_filters()` | `Option<&dyn CodecFilterRegistry>` | Register packet-level filters |
| `transport_filters()` | `Option<&dyn TransportFilterRegistry>` | Register TCP-level filters |
| `register_limbo_handler()` | — | Register a limbo handler |
| `register_config_provider()` | — | Register a dynamic config provider |
| `plugin_id()` | `&str` | This plugin's ID |

The `_handle()` variants return `Arc` so you can move them into event handlers, scheduled tasks, or any `'static` closure:

```rust
let registry = ctx.player_registry_handle();
ctx.scheduler().interval(
    std::time::Duration::from_secs(60),
    Box::new(move || {
        let count = registry.online_count();
        tracing::info!("{count} players online");
    }),
);
```

## PlayerRegistry

Tracks every player connected to the proxy. Players are returned as `Arc<dyn Player>`.

```rust
let registry = ctx.player_registry();

// Find by username (case-insensitive)
if let Some(player) = registry.get_player("Notch") {
    let _ = player.send_message(Component::text("Hello!"));
}

// Find by UUID
if let Some(player) = registry.get_player_by_uuid(&uuid) {
    tracing::info!("Found {}", player.profile().username);
}

// Find by session ID
if let Some(player) = registry.get_player_by_id(player_id) {
    tracing::info!("Player on {:?}", player.current_server());
}

// All players on a specific server
let lobby_players = registry.get_players_on_server(&ServerId::new("lobby"));

// Totals
let total = registry.online_count();
let on_lobby = registry.online_count_on(&ServerId::new("lobby"));
```

| Method | Returns | Description |
|--------|---------|-------------|
| `get_player(username)` | `Option<Arc<dyn Player>>` | Lookup by username (case-insensitive) |
| `get_player_by_uuid(uuid)` | `Option<Arc<dyn Player>>` | Lookup by Mojang UUID |
| `get_player_by_id(id)` | `Option<Arc<dyn Player>>` | Lookup by session `PlayerId` |
| `get_players_on_server(server)` | `Vec<Arc<dyn Player>>` | All players on a backend server |
| `get_all_players()` | `Vec<Arc<dyn Player>>` | Every connected player |
| `online_count()` | `usize` | Total connected player count |
| `online_count_on(server)` | `usize` | Player count on a specific server |

### The Player trait

Each `Arc<dyn Player>` exposes identity, connection state, and actions:

```rust
let player: Arc<dyn Player> = registry.get_player("Steve").unwrap();

// Identity
let id: PlayerId = player.id();
let profile: &GameProfile = player.profile();
let version: ProtocolVersion = player.protocol_version();
let addr: SocketAddr = player.remote_addr();
let server: Option<ServerId> = player.current_server();

// State
let connected: bool = player.is_connected();
let active: bool = player.is_active();

// Actions (require active proxy mode)
player.send_message(Component::text("Hi").color("green"))?;
player.send_title(TitleData::new(
    Component::text("Welcome").color("gold"),
    Component::text("Enjoy your stay"),
))?;
player.send_action_bar(Component::text("Action bar text"))?;
player.send_packet(raw_packet)?;
player.switch_server(ServerId::new("survival")).await?;

// Always works regardless of proxy mode
player.disconnect(Component::text("Goodbye")).await;
```

::: warning
Methods like `send_message`, `send_title`, `send_action_bar`, `send_packet`, and `switch_server` only work in active proxy modes (ClientOnly, Offline, ServerOnly). In passive modes (Passthrough, ZeroCopy), they return `Err(PlayerError::NotActive)`. Check `player.is_active()` first.
:::

## Scheduler

Runs delayed one-shot tasks and recurring interval tasks on the proxy's async runtime.

```rust
use std::time::Duration;

// One-shot: runs once after 5 seconds
let handle = ctx.scheduler().delay(
    Duration::from_secs(5),
    Box::new(|| {
        tracing::info!("5 seconds have passed");
    }),
);

// Recurring: runs every 30 seconds
let registry = ctx.player_registry_handle();
let interval_handle = ctx.scheduler().interval(
    Duration::from_secs(30),
    Box::new(move || {
        tracing::info!("{} players online", registry.online_count());
    }),
);

// Cancel either type of task
ctx.scheduler().cancel(handle);
ctx.scheduler().cancel(interval_handle);
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `delay` | `(Duration, Box<dyn FnOnce() + Send>) -> TaskHandle` | Run once after a delay |
| `interval` | `(Duration, Box<dyn Fn() + Send + Sync>) -> TaskHandle` | Run repeatedly at fixed intervals |
| `cancel` | `(TaskHandle)` | Cancel a scheduled task |

`TaskHandle` is an opaque ID returned by `delay` and `interval`. Store it if you need to cancel the task later.

## ServerManager

Query and control backend server lifecycle.

```rust
let manager = ctx.server_manager();

// Check a server's state
if let Some(state) = manager.get_state(&ServerId::new("survival")) {
    tracing::info!("survival is {:?}", state);
}

// Start or stop a server
manager.start(&ServerId::new("survival")).await?;
manager.stop(&ServerId::new("survival")).await?;

// List all servers
for (id, state) in manager.get_all_servers() {
    tracing::info!("{}: {:?}", id, state);
}

// React to state changes
let handle = manager.on_state_change(Box::new(|server, old, new| {
    tracing::info!("{server}: {old:?} -> {new:?}");
}));
```

`ServerState` has these variants:

| Variant | Meaning |
|---------|---------|
| `Online` | Accepting connections |
| `Offline` | Not running |
| `Starting` | In the process of starting |
| `Stopping` | In the process of stopping |
| `Sleeping` | Sleeping, can be woken on demand |
| `Crashed` | Server has crashed |

`ServerState` is `#[non_exhaustive]`, so always include a wildcard arm in match expressions.

## BanService

Manage player bans by IP, username, or UUID.

```rust
use std::time::Duration;

let bans = ctx.ban_service();

// Permanent ban by username
bans.ban(
    BanTarget::Username("griefer".into()),
    Some("Griefing".into()),
    None, // permanent
).await?;

// Temporary ban by IP (1 hour)
bans.ban(
    BanTarget::Ip("1.2.3.4".parse().unwrap()),
    Some("Spam".into()),
    Some(Duration::from_secs(3600)),
).await?;

// Ban by UUID
bans.ban(
    BanTarget::Uuid(uuid),
    None,
    None,
).await?;

// Check and remove bans
let is_banned = bans.is_banned(&BanTarget::Username("griefer".into())).await?;
let entry = bans.get_ban(&BanTarget::Username("griefer".into())).await?;
let removed = bans.unban(&BanTarget::Username("griefer".into())).await?;

// List all active bans
let all_bans = bans.get_all_bans().await?;
```

`BanTarget` variants: `Ip(IpAddr)`, `Username(String)`, `Uuid(uuid::Uuid)`.

`BanEntry` contains the `target`, `reason`, `expires_at`, `created_at`, and `source` fields. Use `entry.is_expired()`, `entry.is_permanent()`, and `entry.remaining()` to inspect ban state.

## ConfigService

Read-only access to proxy configuration.

```rust
let config = ctx.config_service();

// Get a specific server's config
if let Some(server) = config.get_server_config(&ServerId::new("lobby")) {
    tracing::info!("lobby domains: {:?}", server.domains);
    tracing::info!("proxy mode: {:?}", server.proxy_mode);
    tracing::info!("max players: {}", server.max_players);
}

// List all server configs
for server in config.get_all_server_configs() {
    tracing::info!("{}: {} domains", server.id, server.domains.len());
}

// Read arbitrary config values
if let Some(val) = config.get_value("some.key") {
    tracing::info!("config value: {val}");
}
```

`ServerConfig` fields:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `ServerId` | Server identifier |
| `network` | `Option<String>` | Network group (servers in the same network can switch between each other) |
| `addresses` | `Vec<ServerAddress>` | Backend addresses |
| `domains` | `Vec<String>` | Domains that route to this server |
| `proxy_mode` | `ProxyMode` | Passthrough, ZeroCopy, ClientOnly, Offline, or ServerOnly |
| `limbo_handlers` | `Vec<String>` | Ordered limbo handler names |
| `max_players` | `u32` | Max players (0 = unlimited) |
| `disconnect_message` | `Option<String>` | Message when backend is unreachable |
| `send_proxy_protocol` | `bool` | Whether PROXY protocol is sent to backend |
| `has_server_manager` | `bool` | Whether auto start/stop is configured |

## CommandManager

Register commands that players (or the console) can execute.

```rust
ctx.command_manager().register(
    "hello",             // command name
    &["hi", "hey"],      // aliases
    "Says hello",        // description
    Box::new(HelloCommand),
);

// Later, to remove it:
ctx.command_manager().unregister("hello");
```

Implement `CommandHandler` for your command struct:

```rust
struct HelloCommand;

impl CommandHandler for HelloCommand {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        player_registry: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if let Some(id) = ctx.player_id {
                if let Some(player) = player_registry.get_player_by_id(id) {
                    let _ = player.send_message(
                        Component::text("Hello!").color("gold"),
                    );
                }
            }
        })
    }

    fn tab_complete(&self, _partial_args: &[&str]) -> Vec<String> {
        vec!["world".into(), "proxy".into()]
    }
}
```

`CommandContext` provides `player_id` (None for console commands), `args` (split by whitespace), and `raw` (the full command string).

## EventBus

Subscribe to proxy events using typed handlers. See the [Events page](./events.md) for the full list of available events.

```rust
// Synchronous handler
ctx.event_bus().subscribe::<PostLoginEvent, _>(
    EventPriority::NORMAL,
    |event| {
        tracing::info!("{} joined", event.profile.username);
    },
);

// Async handler
ctx.event_bus().subscribe_async::<PostLoginEvent, _>(
    EventPriority::EARLY,
    |event| {
        let username = event.profile.username.clone();
        Box::pin(async move {
            tracing::info!("{username} joined (async handler)");
        })
    },
);
```

Priority levels control execution order (lowest value runs first):

| Constant | Value | Use case |
|----------|-------|----------|
| `EventPriority::FIRST` | 0 | Security checks, logging |
| `EventPriority::EARLY` | 64 | Pre-processing |
| `EventPriority::NORMAL` | 128 | Default |
| `EventPriority::LATE` | 192 | Post-processing |
| `EventPriority::LAST` | 255 | Monitoring, final overrides |

You can also use `EventPriority::custom(value)` for fine-grained control.

## PluginConfigProvider

Plugins can supply server configurations from external sources (databases, APIs, service discovery). Register a provider during `on_enable`:

```rust
ctx.register_config_provider(Box::new(MyProvider));
```

Implement the `PluginConfigProvider` trait:

```rust
struct MyProvider;

impl PluginConfigProvider for MyProvider {
    fn provider_type(&self) -> &str { "my_api" }

    fn load_initial(&self) -> BoxFuture<'_, Result<Vec<ServerConfig>, PluginError>> {
        Box::pin(async {
            // Fetch initial configs from your source
            Ok(vec![])
        })
    }

    fn watch(
        &self,
        sender: Box<dyn PluginProviderSender>,
    ) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async move {
            while !sender.is_shutdown() {
                // Poll for changes, emit events:
                // sender.send(PluginProviderEvent::Added(config)).await;
                // sender.send(PluginProviderEvent::Updated(config)).await;
                // sender.send(PluginProviderEvent::Removed(server_id)).await;
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
            Ok(())
        })
    }
}
```

The proxy calls `load_initial` once after all plugins are enabled, then spawns `watch` in a background task. Use the `PluginProviderSender` to emit `Added`, `Updated`, or `Removed` events as configurations change.

## Prelude

Import everything you need with a single `use` statement:

```rust
use infrarust_api::prelude::*;
```

This brings in all the types, traits, events, services, and error types covered on this page, plus `Arc` from the standard library.
