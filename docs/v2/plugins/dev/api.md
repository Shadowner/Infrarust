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
| `scheduler()` | `&dyn Scheduler` | Schedule delayed, recurring and async tasks |
| `scheduler_handle()` | `Arc<dyn Scheduler>` | Cloneable handle for closures |
| `services()` | `&dyn ServiceRegistry` | Provide an API to other plugins or use theirs, see [Sharing services](./services) |
| `services_handle()` | `Arc<dyn ServiceRegistry>` | Cloneable handle for closures |
| `plugin_registry()` | `&dyn PluginRegistry` | Read-only view of loaded plugins |
| `plugin_registry_handle()` | `Arc<dyn PluginRegistry>` | Cloneable handle for closures |
| `codec_filters()` | `Option<&dyn CodecFilterRegistry>` | Register packet-level filters (needs the `CodecFilter` capability) |
| `transport_filters()` | `Option<&dyn TransportFilterRegistry>` | Register TCP-level filters (needs the `TransportFilter` capability) |
| `register_limbo_handler(handler)` | `Result<LimboHandlerRegistration, LimboHandlerError>` | Register a limbo handler, see [Limbo handlers](#limbo-handlers) |
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
| `channel_registrar()` | `&dyn ChannelRegistrar` | Register the plugin message channels this plugin listens on, removed when it is disabled. See [Plugin messaging](./messaging) |
| `server_messenger()` | `Arc<dyn ServerMessenger>` | Send a plugin message to a backend server through a player on it. See [Plugin messaging](./messaging#messages-to-a-server) |

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
player.send_plugin_message(&ChannelId::modern("myplugin:main")?, Bytes::from_static(b"hi"))?;
player.send_plugin_message_to_backend(&ChannelId::bungeecord(), request)?;

// Servers: connect waits for the outcome, switch_server does not
let result: ConnectionResult = player.connect(ServerId::new("survival")).await?;
player.switch_server(ServerId::new("survival")).await?;

// Tab list, titles and boss bars
player.set_player_list_header_footer(
    Component::text("My Network").color("gold"),
    Component::text("play.example.com"),
)?;
player.clear_title(true)?;
let bar: BossBarHandle = player.show_boss_bar(
    BossBar::new(Component::text("Event starts soon")).progress(0.25),
)?;
bar.set_progress(0.5)?;
bar.hide()?;

// Resource packs, transfers and cookies
player.send_resource_pack(
    ResourcePackRequest::new("https://cdn.example.com/pack.zip")
        .hash("2e1a4ab4c1f7d1d7a7e9c4c9a16f1b1a0c3d2f10")
        .required(true),
)?;
player.remove_resource_pack(None)?;
player.transfer("eu.example.com", 25565).await?;
player.store_cookie("myplugin:ticket", Bytes::from_static(b"abc"))?;
let ticket: Option<Bytes> = player.request_cookie("myplugin:ticket").await?;

// What the client said about itself (None or empty in the forwarding modes)
let brand: Option<String> = player.client_brand();
let settings: Option<ClientSettings> = player.settings();
let channels: Vec<String> = player.known_channels();
let ping: Option<Duration> = player.ping();
let host: Option<String> = player.virtual_host();

// Always works regardless of proxy mode
player.disconnect(Component::text("Goodbye")).await;
```

`has_permission()` asks the active permission provider's checker for the node, then falls back to the node's registered default, then denies. There are no permission levels: an admin is a player holding `infrarust.admin` (`ADMIN_PERMISSION`). `refresh_permissions()` asks the provider for a new checker and sends this player a rebuilt command tree. See [Permissions](./permissions).

::: warning
Every action except `disconnect` only works when the player is on an active proxy path, which means `ClientOnly` or `Offline` mode: `send_message`, `send_title`, `send_action_bar`, `send_packet`, `send_plugin_message`, `send_plugin_message_to_backend`, `switch_server`, `connect`, `set_player_list_header_footer`, `clear_title`, `show_boss_bar`, `send_resource_pack`, `remove_resource_pack`, `transfer`, `store_cookie` and `request_cookie`. On passive paths (`Passthrough`, `ZeroCopy`, `ServerOnly`) the proxy only copies bytes between client and backend, and they return `Err(PlayerError::NotActive)`. Check `player.is_active()` first. `disconnect` always works.
:::

`client_brand()`, `settings()` and `known_channels()` hold what the client sent on `minecraft:brand`, in its Client Information packet and on `minecraft:register`. `virtual_host()` is the domain the client connected to, known in every proxy mode. `ping()` is the last keepalive round trip the proxy measured. The proxy sends the settings, the brand and the channels again to every server a switch reaches. See [Plugin messaging](./messaging#client-state).

`send_plugin_message` and `send_plugin_message_to_backend` send on a channel, picking the configuration or play packet for the phase the connection is in, and return `PlayerError::MessageTooLarge` over 1 MiB (to the client) or 32767 bytes (to the backend), and `PlayerError::NoBackend` when the player is not on a backend. See [Plugin messaging](./messaging#sending-plugin-messages) for when they are delivered.

#### When actions reach the client

On an active path, the proxy delivers each action according to where the player is:

- **Still logging in or in the configuration phase (1.20.2+).** Messages, titles, `clear_title`, action bars, raw packets, the tab list header and footer, boss bars, and `switch_server` and `connect` requests wait in order and go out right after the player's `JoinGame`. A client in these phases cannot read play packets, so nothing is sent early.
- **Resource packs, transfers and cookies** have a configuration packet as well as a play packet. They go out as soon as the client is in the configuration phase or in game, with the packet for that phase. During the login, and while a server switch runs its configuration phase, they wait.
- **In game.** Actions go out as soon as the proxy's connection loop picks them up.
- **In limbo.** Same as in game. The player is in play state, so messages and titles show at once. `switch_server` and `connect` take the player out of limbo and send them to that server.

`disconnect(reason)` never waits in that queue. The reason is sent in the packet the client expects at that moment: the login disconnect during login, the configuration disconnect during the configuration phase, and the play disconnect in game or in limbo. In game, messages queued before the kick are sent before it; actions still waiting for `JoinGame` are dropped. `disconnect` returns right away. If the player already has a full backlog of pending actions, the connection is closed without the reason.

On a passive path (`Passthrough`, `ZeroCopy`, `ServerOnly`), the proxy only copies bytes between client and backend and cannot add a packet to the stream. `disconnect` closes both connections and the reason is not shown: the client sees a plain connection loss.

#### Connecting to a server

`connect(target)` sends the player to another server and resolves once the switch is over, with a `ConnectionResult`:

| Result | When |
|--------|------|
| `Success` | The player joined `target`, or the server a [`ServerPreConnectEvent`](./events#serverpreconnectevent) listener redirected them to |
| `AlreadyConnected` | The player was already on `target`, or a listener redirected them to the server they are on |
| `Denied(reason)` | A `ServerPreConnectEvent` listener denied the switch. The player stays where they are and sees `reason` in chat |
| `Failed(reason)` | The server could not be reached, refused the login, or disconnected the player during the switch. `reason` is the server's disconnect reason, or its `disconnect_message` when it gave none. What happens to the player is up to [`KickedFromServerEvent`](./events#kickedfromserverevent); by default they stay |
| `Cancelled` | The switch ended without an outcome: the player left or was kicked, the proxy shut down, a listener sent the player to limbo instead, or the request was dropped |

`is_success()` is `true` for `Success` and `AlreadyConnected`. `connect` returns `Err(PlayerError::NotActive)` on a passive path and `Err(PlayerError::Disconnected)` when the player has left. It has no timeout of its own; wrap it in `tokio::time::timeout` to bound the wait.

`switch_server(target)` queues the same request and returns as soon as it is queued. It stays for code that does not need the outcome, like the proxy's `/server` and `/send` commands. Both fire `ServerPreConnectEvent` with the cause `switch`. When several `connect` calls wait for the same server, one switch there settles all of them.

```rust
match player.connect(ServerId::new("minigames")).await? {
    ConnectionResult::Success | ConnectionResult::AlreadyConnected => {}
    ConnectionResult::Failed(reason) => {
        player.send_message(Component::error("Minigames are down: ").append(reason))?;
    }
    _ => {}
}
```

#### Tab list, titles and boss bars

`set_player_list_header_footer(header, footer)` sets the text above and below the player list (1.8 and later, `Err(PlayerError::Unsupported)` on 1.7). The proxy keeps the last header and footer a plugin set and sends them again after every server switch, right after the new server's `JoinGame`: from 1.20.2 the configuration phase of a switch clears them on the client, and before that the new server may send its own. A backend can still replace them; the last packet the client gets wins.

`clear_title(reset)` removes the title on screen. With `reset`, the fade times go back to the client's defaults too. From 1.17 it sends Clear Titles; from 1.8 to 1.16.4 it sends the Title packet with the hide action, or the reset action when `reset` is set. On 1.7, which has no titles, it does nothing.

`show_boss_bar(bar)` shows a boss bar owned by the proxy and returns a `BossBarHandle` (1.9 and later, `Err(PlayerError::Unsupported)` on 1.8 and older). The proxy gives each bar a random UUID of its own, so it never clashes with the bars a backend shows.

```rust
let bar = player.show_boss_bar(
    BossBar::new(Component::text("Double XP"))
        .progress(1.0)
        .color(BossBarColor::Purple)
        .overlay(BossBarOverlay::Notched10)
        .flags(BossBarFlags::NONE.with(BossBarFlags::DARKEN_SCREEN)),
)?;
bar.set_title(Component::text("Double XP: 5 minutes left"))?;
bar.set_progress(0.5)?;
bar.set_style(BossBarColor::Red, BossBarOverlay::Progress)?;
bar.hide()?;
```

| `BossBarHandle` method | Sends |
|------------------------|-------|
| `set_title(component)` | Update title |
| `set_progress(f32)` | Update health. The value is clamped to `0.0..=1.0` |
| `set_style(color, overlay)` | Update style |
| `set_flags(flags)` | Update flags: `DARKEN_SCREEN`, `PLAY_BOSS_MUSIC`, `CREATE_WORLD_FOG` |
| `update(BossBarUpdate)` | Any of the above |
| `hide()` | Remove |

The bar stays until `hide()`; dropping the handle does not remove it, and clones drive the same bar. After `hide()`, the handle returns `Err(PlayerError::InvalidArgument)`. When the player leaves, the proxy forgets their bars and the handle returns `Err(PlayerError::Disconnected)`.

Server switches:

- **Before 1.20.2** the client keeps its boss bars across a `JoinGame`. The proxy's bars stay on screen, and the proxy removes the bars the previous server showed, which it tracks as they pass, so they do not linger.
- **From 1.20.2** the configuration phase of a switch clears every bar. The proxy shows its own bars again, with their latest title, progress, style and flags, right after the new server's `JoinGame`.

#### Resource packs

`send_resource_pack(pack)` asks the client to download and apply a resource pack:

```rust
let pack = ResourcePackRequest::new("https://cdn.example.com/event.zip")
    .hash("2e1a4ab4c1f7d1d7a7e9c4c9a16f1b1a0c3d2f10")
    .required(true)
    .prompt(Component::text("This event needs its textures"));
let pack_id = pack.id;
player.send_resource_pack(pack)?;
// later
player.remove_resource_pack(Some(pack_id))?;
```

| Field | Description |
|-------|-------------|
| `id` | The pack's UUID. `new()` picks a random one; set your own with `.id(uuid)` |
| `url` | Where the client downloads the pack, at most 32767 characters |
| `hash` | The SHA-1 of the file as 40 hexadecimal characters, or `None` to skip the check |
| `required` | The client is told the server requires the pack, and a vanilla client leaves the server when the player declines it |
| `prompt` | A message on the client's prompt |

A hash that is not 40 hexadecimal characters, or a longer URL, returns `Err(PlayerError::InvalidArgument)`: the client would fail to read the packet and disconnect.

| Client | Sent | `remove_resource_pack` |
|--------|------|------------------------|
| 1.20.3 and later | Add Resource Pack with `id`. Packs stack on the client | Remove Resource Pack: one pack with `Some(id)`, every server pack with `None` |
| 1.17 to 1.20.2 | Resource Pack Send, which replaces the previous pack. `id` is not sent; the proxy keeps it to report statuses | `Err(PlayerError::Unsupported)`: the protocol has no way to remove a pack before 1.20.3 |
| 1.8 to 1.16.4 | The same, without `required` and `prompt`, which these versions lack | `Err(PlayerError::Unsupported)` |
| 1.7 | Nothing: `Err(PlayerError::Unsupported)` | `Err(PlayerError::Unsupported)` |

Every answer the client gives, for a pack from the proxy or from a backend, posts a [`PlayerResourcePackStatusEvent`](./events#playerresourcepackstatusevent). Answers to the proxy's packs stay on the proxy; answers to a backend's packs are forwarded to it unchanged. From 1.20.3 the proxy tells them apart by pack UUID. Before 1.20.3 the answers carry no UUID, so the proxy matches them in the order the packs reached the client: an answer belongs to the oldest pack that has not reached a final status yet.

#### Transfers

`transfer(host, port)` sends the player to another server or proxy with the Transfer packet (1.20.5 and later, `Err(PlayerError::Unsupported)` before). The client closes its connection to Infrarust and connects to `host:port` with the transfer intent, keeping its [cookies](#cookies). The target must accept transfers: `accepts-transfers=true` on a vanilla server.

Before the packet is sent, the proxy fires [`PreTransferEvent`](./events#pretransferevent) with the origin `Plugin`. A listener that denies it makes `transfer` return `Err(PlayerError::Denied(reason))` and nothing is sent; a redirect sends the listener's destination instead. `transfer` returns once the packet is queued. An empty host, or one over 32767 characters, returns `Err(PlayerError::InvalidArgument)`.

A Transfer packet a backend sends goes through the same event, with the origin `Backend`: allowed, it reaches the client unchanged; redirected, the proxy rewrites the destination; denied, the proxy drops it.

#### Cookies

`store_cookie(key, data)` stores up to 5120 bytes on the client, and `request_cookie(key)` reads them back (1.20.5 and later, `Err(PlayerError::Unsupported)` before). The client keeps its cookies while it moves between servers with transfers, and forgets them when the game closes.

```rust
player.store_cookie("myplugin:ticket", Bytes::from(ticket.to_bytes()))?;

// after a transfer, on the proxy the player lands on
match player.request_cookie("myplugin:ticket").await? {
    Some(data) => verify(&data),
    None => tracing::info!("no ticket"),
}
```

A key is an identifier: `namespace:path` in lowercase letters, digits, `_`, `-` and `.`, with `/` also allowed in the path. A key without a namespace gets `minecraft:`, as the client would give it. Another key, or data over 5120 bytes, returns `Err(PlayerError::InvalidArgument)`.

`request_cookie` resolves with `Some(data)`, or `None` when the client has no cookie under that key. It returns `Err(PlayerError::Disconnected)` if the player leaves before answering. It has no timeout of its own: a vanilla client always answers, and `tokio::time::timeout` bounds the wait when you need to. The proxy matches answers to requests by key and in order, so a plugin and a backend can ask for the same cookie at the same time: each gets its own answer, and only the backend's reaches the backend.

## Scheduler

Runs one-shot, recurring and async tasks on the proxy's async runtime. Every task a plugin schedules through its context belongs to that plugin: it is cancelled when the plugin is disabled, and a plugin cannot cancel another plugin's task.

```rust
use std::time::Duration;

// One-shot: runs once after 5 seconds
let handle = ctx.scheduler().delay(
    Duration::from_secs(5),
    Box::new(|| {
        tracing::info!("5 seconds have passed");
    }),
);

// Recurring at a fixed rate: runs every 30 seconds
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

The async variants take futures, so a task can await storage, HTTP calls or other services:

```rust
let storage = self.storage.clone();
ctx.scheduler().repeat(
    Duration::from_secs(60),
    None,
    Box::new(move || {
        let storage = storage.clone();
        Box::pin(async move {
            if let Err(e) = storage.flush().await {
                tracing::error!("auto-save failed: {e}");
            }
        })
    }),
);

let bans = ctx.ban_service_handle();
ctx.scheduler().spawn(Box::pin(async move {
    let _ = bans.list_all().await;
}));
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `delay` | `(Duration, Box<dyn FnOnce() + Send>) -> TaskHandle` | Run once after a delay |
| `interval` | `(Duration, Box<dyn Fn() + Send + Sync>) -> TaskHandle` | Run at a fixed rate, first run after one period |
| `interval_with_delay` | `(Duration, Duration, Box<dyn Fn() + Send + Sync>) -> TaskHandle` | Run at a fixed rate, first run after the delay |
| `spawn` | `(BoxFuture<'static, ()>) -> TaskHandle` | Run a future now |
| `delay_async` | `(Duration, AsyncTask) -> TaskHandle` | Build and await a future once, after a delay |
| `repeat` | `(Duration, Option<Duration>, RepeatingTask) -> TaskHandle` | Await a new future every period, never two at once |
| `spawn_blocking` | `(Box<dyn FnOnce() + Send>) -> TaskHandle` | Run blocking code on the blocking thread pool |
| `cancel` | `(TaskHandle)` | Cancel a task |

`AsyncTask` is `Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>` and `RepeatingTask` is `Box<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>`.

How each kind runs:

- `repeat` waits `period` after a run finishes before starting the next one, so runs never overlap and a slow run pushes the next one back. The first run starts after `initial_delay`, or after one `period` when it is `None`. `Some(Duration::ZERO)` runs it right away.
- `interval` and `interval_with_delay` run at a fixed rate. The closure is synchronous, so keep it short; a tick missed because the runtime was busy is skipped, not replayed.
- A period below 1 ms is raised to 1 ms.
- A task that panics is logged with the plugin ID. The panic ends that run only: the scheduler keeps working, and a repeating task runs again at its next period.
- `cancel` stops a sync task before its next run. An async task is dropped at its next `.await`. A `spawn_blocking` closure that already started runs to completion.

`TaskHandle` is an opaque ID. The proxy forgets it once the task finishes, so a finished one-shot task leaves nothing behind. Cancelling a finished task, or a task scheduled by another plugin, does nothing.

## Limbo handlers

`register_limbo_handler` makes a [limbo handler](./architecture#layer-4-limbohandler) available to every server whose `limbo_handlers` list names it. It works at any time, not only during `on_enable`: a server resolves its handlers when a player arrives, so a handler registered later is used by the next player.

```rust
let registration = ctx.register_limbo_handler(Box::new(QueueHandler::new()))?;

// later, to stop gating players
registration.unregister();
```

| Outcome | Result |
|---------|--------|
| Registered | `Ok(LimboHandlerRegistration)` |
| The plugin lacks the `Limbo` capability | `Err(LimboHandlerError::MissingCapability)` |
| Another registration uses the same name | `Err(LimboHandlerError::NameTaken { name, owner })`. The first one stays |

`LimboHandlerError` converts into `PluginError::InitFailed`, so `?` works inside `on_enable`.

Dropping the `LimboHandlerRegistration` keeps the handler registered. `unregister()` removes it and returns `false` if it was already gone. Disabling the plugin removes all of its handlers.

A removed handler fails closed. Every player it holds (its `on_player_enter` returned `Hold` or `HoldWithTimeout` and has not completed) is released with `HandlerResult::unavailable()`, a denial with the text "Limbo handler unavailable". A player who reaches the handler afterwards through a chain resolved before the removal is denied the same way. Players who arrive later skip the missing name, as they do for any name no plugin registered.

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

This brings in the common types, traits, events, services, and error types covered on this page, including `ServiceRegistryExt` for `provide` and `get`, plus `Arc` from the standard library. A few items live outside the prelude: `DefaultPermissionChecker`, `AllPermissionsChecker` and `normalize_node` are in `infrarust_api::permissions`, and `ProxyInfo` and `PluginRegistry` are in `infrarust_api::services`. Import those directly when you need them.
