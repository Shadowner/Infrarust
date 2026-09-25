---
title: Plugin API Reference
description: Reference for the Plugin trait, PluginContext, services, and types available to native Infrarust plugins.
outline: [2, 3]
---

# Plugin API Reference

Every native plugin implements the `Plugin` trait and receives a `PluginContext` during `on_enable`. The context is your access point to the proxy's services: player registry, scheduler, server manager, ban service, config service, event bus, command manager, plugin registry, and read-only proxy info.

All services are trait objects, and the proxy is the sole implementor. You reach them through the context and, where needed, capture `Arc` handles for use inside closures.

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

Import everything with `use infrarust_api::prelude::*;`. All crates are at version `v2.0.0-beta.3` (Rust edition 2024, MSRV 1.94).

## The Plugin trait

Every plugin implements `Plugin`, which has two methods. `metadata` returns a `PluginMetadata` describing the plugin, and `on_enable` is the entry point where you register listeners, commands, and handlers. `on_disable` has a default implementation that does nothing, so override it only when you need to clean up.

```rust
use infrarust_api::prelude::*;

struct MyPlugin;

impl Plugin for MyPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("my_plugin", "My Plugin", "1.0.0")
            .author("Alice")
            .description("A cool plugin")
            .depends_on("core_plugin")
    }

    fn on_enable<'a>(&'a self, ctx: &'a dyn PluginContext) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async move {
            // register event listeners, commands, etc.
            Ok(())
        })
    }

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async { Ok(()) })
    }
}
```

`PluginMetadata` is a builder. `new(id, name, version)` takes the three required fields, and the chainable methods add the rest:

| Builder method | Field | Description |
|----------------|-------|-------------|
| `new(id, name, version)` | `id`, `name`, `version` | `id` is a `snake_case` identifier; `version` is a semver string |
| `author(name)` | `authors` | Adds an author (call once per author) |
| `description(text)` | `description` | Sets an optional description |
| `depends_on(id)` | `dependencies` | Adds a required dependency on another plugin |
| `optional_dependency(id)` | `dependencies` | Adds a dependency the plugin can run without |

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
| `config_service()` | `&dyn ConfigService` | Read proxy and server configuration, and rewrite the global one with the `ConfigWrite` capability |
| `config_service_handle()` | `Arc<dyn ConfigService>` | Cloneable handle for closures |
| `load_balancer_service()` | `&dyn LoadBalancerService` | Read per-address backend status, drain or reset an address |
| `load_balancer_service_handle()` | `Arc<dyn LoadBalancerService>` | Cloneable handle for closures |
| `command_manager()` | `&dyn CommandManager` | Register and unregister commands |
| `command_manager_handle()` | `Arc<dyn CommandManager>` | Owned handle for registering commands after `on_enable` |
| `scheduler()` | `&dyn Scheduler` | Schedule delayed and recurring tasks |
| `plugin_registry()` | `&dyn PluginRegistry` | Read-only view of loaded plugins |
| `plugin_registry_handle()` | `Arc<dyn PluginRegistry>` | Cloneable handle for closures |
| `codec_filters()` | `Option<&dyn CodecFilterRegistry>` | Register packet-level filters (needs the `CodecFilter` capability) |
| `transport_filters()` | `Option<&dyn TransportFilterRegistry>` | Register TCP-level filters (needs the `TransportFilter` capability) |
| `register_limbo_handler(handler)` | `()` | Register a limbo handler |
| `register_config_provider(provider)` | `()` | Register a dynamic config provider |
| `register_ban_provider(provider)` | `Result<(), BanProviderRejected>` | Become the ban provider. Needs `ban-provider` and `[ban] provider` naming this plugin, see [Bans](./bans) |
| `register_permission_provider(provider)` | `Result<(), PermissionProviderRejected>` | Become the permission provider. Needs `permission-provider` and `[permissions] provider` naming this plugin, see [Permissions](./permissions) |
| `register_permission_node(node)` | `Result<(), PermissionNodeError>` | Register a node this plugin checks, with its default. Removed when the plugin is disabled |
| `permission_nodes()` | `Vec<PermissionNodeInfo>` | Every registered node and the plugin that owns it |
| `proxy_info()` | `&ProxyInfo` | Read-only proxy version and runtime settings |
| `capabilities()` | `&CapabilitySet` | Capabilities granted to this plugin |
| `data_dir()` | `PathBuf` | This plugin's data directory, `<plugins_dir>/<plugin_id>`, created if missing |
| `proxy_shutdown()` | `CancellationToken` | Token that fires when the proxy shuts down |
| `plugin_id()` | `&str` | This plugin's ID |

`codec_filters()` and `transport_filters()` return `None` unless the plugin holds the matching capability in its Infrarust config, with the transport filter capability reserved for trusted native plugins.

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

// Every player connecting from the same address (alt accounts)
let alts = registry.get_players_by_ip(ip);

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
| `get_players_by_ip(ip)` | `Vec<Arc<dyn Player>>` | Every player whose client connects from `ip` |
| `get_players_on_server(server)` | `Vec<Arc<dyn Player>>` | All players on a backend server |
| `get_all_players()` | `Vec<Arc<dyn Player>>` | Every connected player |
| `online_count()` | `usize` | Total connected player count |
| `online_count_on(server)` | `usize` | Player count on a specific server |

Lookups by username, UUID, session ID and IP use indexes and never scan the online players. A player is findable from the moment `PostLoginEvent` fires until its `DisconnectEvent` has been handled.

`get_player` ignores case: `"notch"`, `"Notch"` and `"NOTCH"` find the same player. In offline mode, two players can be online whose names differ only by case. When that happens, the one spelled exactly like the argument wins, and otherwise either one is returned.

`get_players_by_ip` matches the address in `Player::remote_addr()`. With `receive_proxy_protocol` enabled, that is the client address from the PROXY protocol header, not the load balancer's. An IPv4 address also matches clients seen as IPv4-mapped IPv6 (`::ffff:a.b.c.d`).

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
let connected_at: SystemTime = player.connected_at();

// State
let connected: bool = player.is_connected();
let active: bool = player.is_active();
let online_mode: bool = player.is_online_mode();

// Permissions
let is_admin: bool = player.has_permission(ADMIN_PERMISSION);
let can_do_it: bool = player.has_permission("my_plugin.feature");
player.refresh_permissions().await;

// Actions (require an active proxy mode)
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

`has_permission()` asks the active permission provider's checker for the node, then falls back to the node's registered default, then denies. There are no permission levels: an admin is a player holding `infrarust.admin` (`ADMIN_PERMISSION`). `refresh_permissions()` asks the provider for a new checker and sends this player a rebuilt command tree. See [Permissions](./permissions).

::: warning
`send_message`, `send_title`, `send_action_bar`, `send_packet`, and `switch_server` only work when the player is on an active proxy path, which means `ClientOnly` or `Offline` mode. On passive paths (`Passthrough`, `ZeroCopy`, `ServerOnly`) they return `Err(PlayerError::NotActive)`. Check `player.is_active()` first. `disconnect` always works.
:::

#### When actions reach the client

On an active path, the proxy delivers each action according to where the player is:

- **Still logging in or in the configuration phase (1.20.2+).** Messages, titles, action bars, raw packets and `switch_server` requests wait in order and go out right after the player's `JoinGame`. A client in these phases cannot read play packets, so nothing is sent early.
- **In game.** Actions go out as soon as the proxy's connection loop picks them up.
- **In limbo.** Same as in game. The player is in play state, so messages and titles show at once. `switch_server` takes the player out of limbo and sends them to that server.

`disconnect(reason)` never waits in that queue. The reason is sent in the packet the client expects at that moment: the login disconnect during login, the configuration disconnect during the configuration phase, and the play disconnect in game or in limbo. In game, messages queued before the kick are sent before it; actions still waiting for `JoinGame` are dropped. `disconnect` returns right away. If the player already has a full backlog of pending actions, the connection is closed without the reason.

On a passive path (`Passthrough`, `ZeroCopy`, `ServerOnly`), the proxy only copies bytes between client and backend and cannot add a packet to the stream. `disconnect` closes both connections and the reason is not shown: the client sees a plain connection loss.

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
| `interval` | `(Duration, Box<dyn Fn() + Send + Sync>) -> TaskHandle` | Run repeatedly at a fixed interval |
| `interval_with_delay` | `(Duration, Duration, Box<dyn Fn() + Send + Sync>) -> TaskHandle` | Repeat at a fixed interval, after an initial delay |
| `cancel` | `(TaskHandle)` | Cancel a scheduled task |

`TaskHandle` is an opaque ID returned by `delay`, `interval`, and `interval_with_delay`. Store it if you need to cancel the task later.

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

```

To react to state changes, subscribe to `ServerStateChangeEvent` on the event bus. Like every listener, it is removed automatically when your plugin is disabled:

```rust
use infrarust_api::events::proxy::ServerStateChangeEvent;

ctx.event_bus().subscribe::<ServerStateChangeEvent, _>(
    EventPriority::NORMAL,
    |event| {
        tracing::info!("{}: {:?} -> {:?}", event.server, event.old_state, event.new_state);
    },
);
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

Ban, unban and look up players by IP, IP range, username or UUID. The service forwards to whichever ban provider the operator selected, the built-in one or a plugin's.

```rust
use std::time::Duration;

let bans = ctx.ban_service();

// Permanent ban by username, attributed to this plugin
bans.ban(BanRequest::new(BanTarget::Username("griefer".into())).reason("Griefing"))
    .await?;

// Temporary range ban (1 hour)
bans.ban(
    BanRequest::new(BanTarget::IpRange("203.0.113.0/24".parse()?))
        .reason("Spam")
        .duration(Duration::from_secs(3600)),
)
.await?;

// Look up and remove
let entry = bans.get(&BanTarget::Username("griefer".into())).await?;
let removed = bans.unban(UnbanRequest::new(BanTarget::Username("griefer".into()))).await?;

// One page of active bans, or all of them
let page = bans.list(BanQuery::new().limit(50)).await?;
let all_bans = bans.list_all().await?;
```

`BanEntry` has an `id`, the `target`, `reason`, `source` (a `BanSource`), `created_at` and `expires_at`. Use `entry.is_expired()`, `entry.is_permanent()` and `entry.remaining()` to inspect it.

A plugin can also become the ban provider with `ctx.register_ban_provider(...)`. See [Bans](./bans) for the provider model, request options, sources and events.

## ConfigService

Access to proxy configuration. Everything but the global write is readable by any plugin.

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

`ServerConfig` only carries the fields this crate models. For the rest, read the document:

```rust
// The server's full TOML, whatever provider supplied it
let document = config.get_server_document(&ServerId::new("lobby"));

// Where each server came from: file, docker, plugin:<id>:<type>
for source in config.list_server_sources() {
    tracing::info!("{} from {} (editable: {})", source.id, source.provider_type, source.editable);
}
```

`get_proxy_config_document()` returns `infrarust.toml` with every secret field replaced by `<redacted>`, and `write_proxy_config_document(&toml)` replaces it. `get_effective_proxy_config_document()` returns the same document with the CLI overrides the process was started with applied, and `get_server_document()` redacts server credentials the same way. The write needs the `ConfigWrite` capability: without it the plugin is handed a read-only service that answers `ConfigWriteError::PermissionDenied`, and there is no ungated handle to reach around it. Secret fields the submitted document leaves out or carries redacted keep the value already on disk, so a document read back from `get_proxy_config_document()` can be submitted unchanged. Nothing is applied to the running proxy; the new file takes effect on restart.

## LoadBalancerService

Per-address visibility and maintenance controls for a server's backends. See [Load balancing](../../configuration/load-balancing) for the behavior behind them.

```rust
let lb = ctx.load_balancer_service();
let lobby = ServerId::new("lobby");

for backend in lb.backends(&lobby) {
    tracing::info!(
        "{} is {} ({} connections, weight {}/{})",
        backend.address,
        backend.state.as_str(),
        backend.active_connections,
        backend.effective_weight,
        backend.weight,
    );
}

let address = ServerAddress { host: "10.0.0.2".into(), port: 25565 };

// Take an address out of rotation for maintenance, then put it back
lb.set_drained(&lobby, &address, true)?;
lb.set_drained(&lobby, &address, false)?;

// Clear an ejected address's failure history so it retries now
lb.reset_backend(&lobby, &address)?;
```

`strategy()` names the balancing strategy of a server, and returns `None` for a server that is not routed. `BackendStatus` carries the address, its configured `weight` and the `effective_weight` selection actually uses after the slow-start ramp, its `state` (`healthy`, `probing`, `unhealthy`, `draining`), the live connection count, and the failure history. Both mutations return `LbError` when the server or the address is unknown, so a typo cannot drain something you did not name.

Draining stops new sessions from reaching an address without closing the ones already on it, and it survives the passive health checks underneath. It is not persisted: the proxy rebuilds health state on every start, so a plugin that wants a drain to outlive a restart has to store it and replay it, which is what the admin API does.

## CommandManager

Register commands that players and the console can run. The manager you get from the context is bound to your plugin: it registers commands under your plugin id and can only unregister your own commands.

```rust
let spec = CommandSpec::new("hello")
    .aliases(["hi", "hey"])
    .description("Says hello")
    .permission("hello.use");

match ctx.command_manager().register(spec, Box::new(HelloCommand)) {
    Ok(registration) => tracing::info!("registered /{}", registration.namespaced),
    Err(e) => tracing::warn!("/hello was not registered: {e}"),
}

let _ = ctx.command_manager().unregister("hello");
```

| Method | Returns | Description |
|--------|---------|-------------|
| `register(spec, handler)` | `Result<CommandRegistration, CommandError>` | Register a command; see the conflict rules on the [Commands page](./commands#names-aliases-and-conflicts) |
| `unregister(name)` | `Result<(), CommandError>` | Remove one of your commands by name, alias, or `<plugin_id>:<name>` |
| `list()` | `Vec<CommandInfo>` | Every registered command, built-ins and other plugins included |

Implement `CommandHandler` for your command struct:

```rust
struct HelloCommand;

impl CommandHandler for HelloCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            ctx.source.send_message(Component::text("Hello!").color("gold"));
        })
    }

    fn suggest<'a>(&'a self, _ctx: SuggestContext) -> BoxFuture<'a, Vec<Suggestion>> {
        Box::pin(async { vec![Suggestion::new("world"), Suggestion::new("proxy")] })
    }
}
```

`execute` is required. `suggest` has a default that returns no suggestions.

`CommandContext` provides `source` (a `CommandSource`: a player or the console), `label` (the name or alias typed), `args` (split by whitespace), `raw_args` (everything after the label), and `raw` (the whole command). `CommandSource` offers `name()`, `send_message()`, `has_permission()`, and `player()`. See the [Commands page](./commands) for permission nodes, hidden commands, and the client command tree.

## EventBus

Subscribe to proxy events using typed handlers. See the [Events page](./events) for the full list of available events.

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

    fn load_initial_documents(
        &self,
    ) -> BoxFuture<'_, Result<Vec<ServerDocument>, PluginError>> {
        Box::pin(async {
            Ok(vec![ServerDocument {
                id: ServerId::new("survival"),
                toml: r#"addresses = ["10.0.0.1:25565"]"#.to_string(),
            }])
        })
    }

    fn watch(
        &self,
        sender: Box<dyn PluginProviderSender>,
    ) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async move {
            while !sender.is_shutdown() {
                // Poll for changes, emit events:
                // sender.send(PluginProviderEvent::AddedDocument(doc)).await;
                // sender.send(PluginProviderEvent::UpdatedDocument(doc)).await;
                // sender.send(PluginProviderEvent::Removed(server_id)).await;
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
            Ok(())
        })
    }
}
```

The proxy calls `load_initial` and `load_initial_documents` once after all plugins are enabled, then spawns `watch` in a background task. Use the `PluginProviderSender` to emit `Added`, `Updated`, `AddedDocument`, `UpdatedDocument`, or `Removed` events as configurations change.

### Configs or documents

A provider supplies either form, or both. `ServerConfig` is the projection this crate models, so it reaches `domains`, `addresses`, `proxy_mode`, `limbo_handlers` and little else; a config built that way gets the defaults for everything the struct does not carry.

A `ServerDocument` is raw TOML that the proxy parses against its own full schema, the same one `servers_dir` files use, so it is the only way to reach `balance`, `slow_start`, `[active_health]`, `[motd]`, `[server_manager]`, `[timeouts]` or `ip_filter` from a provider. Its `id` identifies the document within the provider and becomes the server id when the TOML itself sets neither `name` nor `id`. A document that fails to parse or validate is logged and skipped, and the server keeps whatever configuration it already had.

## Prelude

Import everything you need with a single `use` statement:

```rust
use infrarust_api::prelude::*;
```

This brings in the common types, traits, events, services, and error types covered on this page, plus `Arc` from the standard library. A few items live outside the prelude: `DefaultPermissionChecker`, `AllPermissionsChecker` and `normalize_node` are in `infrarust_api::permissions`, and `ProxyInfo` and `PluginRegistry` are in `infrarust_api::services`. Import those directly when you need them.
