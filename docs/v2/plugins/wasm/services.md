---
title: Host Services
description: Query players, manage servers and bans, read config, register commands and codec filters, schedule tasks, log, and build chat components from a WASM plugin.
outline: [2, 3]
---

# Host Services

A WASM plugin reaches the proxy through host services. Each service is a typed accessor on the [`Context`](./api-reference) you receive in `on_enable`, and each one is gated by a [capability](./capabilities). Baseline capabilities are granted to every plugin; opt-in capabilities must be listed in the plugin's TOML `permissions`.

```rust
use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct MyPlugin;

#[plugin(id = "my-plugin", name = "My Plugin")]
impl Plugin for MyPlugin {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        let online = ctx.player_registry().online_count(); // [!code focus]
        info!("{online} players online");                  // [!code focus]
        Ok(())
    }
}
```

The accessors return zero-sized handles, so calling `ctx.player_registry()` is free. Inside a command, scheduled task, or event handler you can also use the unit struct directly (`Players.online_count()`) without holding a `&Context`.

## Capabilities at a glance

| Service | Accessor | Capability | Grant |
| --- | --- | --- | --- |
| Players (read) | `ctx.player_registry()` | `player-read` | baseline |
| Players (write) | `ctx.player_registry()` | `player-write` | baseline |
| `Player::send_packet` | on the `Player` handle | `raw-packet` | opt-in |
| Servers | `ctx.server_manager()` | `server-manage` | opt-in |
| Bans | `ctx.ban_service()` | `ban` | opt-in |
| Config | `ctx.config_service()` | `config-read` | baseline |
| Commands | `ctx.command(name, handler)` | `command` | baseline |
| Codec filters | `Plugin::register_codec_filters` | `codec-filter` | opt-in |
| Scheduler | `ctx.delay` / `ctx.interval` / `ctx.cancel` | `scheduler` | baseline |
| Logging | `info!` family | always available | always |

Opt-in capabilities are listed in the plugin's config block. Capability strings are kebab-case.

```toml
[plugins.my-plugin]
path = "plugins/my_plugin.wasm"
permissions = ["server-manage", "ban", "raw-packet"]
enabled = true
```

:::tip
Native plugins receive every capability. A WASM plugin gets the baseline set plus whatever opt-ins you declare, and nothing else. An unknown or non-grantable capability string is logged and ignored.
:::

:::warning
Calls into `server-manage` and `ban` run under a 30-second host timeout. If a call exceeds it, the result is `ServiceError::Unavailable("host call timed out")`.
:::

## Players

`ctx.player_registry()` returns `Players`. Lookups are baseline (`player-read`); the actions on a `Player` handle are baseline (`player-write`), except `send_packet` which needs `raw-packet`.

```rust
pub fn online_count(&self) -> u32;
pub fn online_count_on(&self, server: &str) -> u32;
pub fn get_by_id(&self, id: u64) -> Option<Player>;
pub fn get_by_name(&self, username: &str) -> Option<Player>;
pub fn get_by_uuid(&self, uuid: &str) -> Option<Player>;
pub fn on_server(&self, server: &str) -> Vec<Player>;
pub fn all(&self) -> Vec<Player>;
```

### The Player handle

A `Player` is a resource owned by the host. Read its state:

| Method | Returns | Meaning |
| --- | --- | --- |
| `id()` | `u64` | Stable per-connection id |
| `profile()` | `GameProfile` | UUID, username, properties |
| `protocol_version()` | `i32` | Client protocol version |
| `remote_addr()` | `String` | Client socket address |
| `current_server()` | `Option<String>` | Backend the player is on |
| `is_connected()` | `bool` | TCP connection is live |
| `is_active()` | `bool` | Player is in an actionable state |
| `is_online_mode()` | `bool` | Authenticated against Mojang |
| `permission_level()` | `PermissionLevel` | `Player` or `Admin` |
| `has_permission(&str)` | `bool` | Single-permission check |
| `connected_at()` | `u64` | Connection time, epoch millis |

Act on the player. `Player` is the binding-generated resource, so the message methods take a `&str` of [component JSON](#building-chat-components). Build it with `Component` and pass `&...into_json()`.

```rust
let player = ctx.player_registry().get_by_name("Notch")
    .ok_or("player not found")?;

player.send_message(&Component::text("Hello").color("gold").into_json())?;
player.send_action_bar(&Component::text("Watch out").into_json())?;
player.switch_server("lobby")?;
player.disconnect(&Component::text("Goodbye").into_json());
```

| Action | Signature | Capability |
| --- | --- | --- |
| `disconnect` | `fn disconnect(&self, reason: &str)` | `player-write` |
| `send_message` | `fn send_message(&self, message: &str) -> Result<(), PlayerError>` | `player-write` |
| `send_title` | `fn send_title(&self, title: &TitleData) -> Result<(), PlayerError>` | `player-write` |
| `send_action_bar` | `fn send_action_bar(&self, message: &str) -> Result<(), PlayerError>` | `player-write` |
| `switch_server` | `fn switch_server(&self, target: &str) -> Result<(), PlayerError>` | `player-write` |
| `send_packet` | `fn send_packet(&self, packet: &RawPacket) -> Result<(), PlayerError>` | `raw-packet` |

:::info
`disconnect` and `switch_server` are dispatched in the background on the host, so they return immediately. `send_packet` without the `raw-packet` capability returns `PlayerError::SendFailed("missing capability: raw-packet")` rather than trapping.
:::

## Servers

`ctx.server_manager()` returns `Servers`. Every method needs the opt-in `server-manage` capability.

```rust
pub fn state(&self, server: &str) -> Option<ServerState>;
pub fn start(&self, server: &str) -> Result<(), ServiceError>;
pub fn stop(&self, server: &str) -> Result<(), ServiceError>;
pub fn all(&self) -> Vec<(String, ServerState)>;
```

`ServerState` is one of `Online`, `Offline`, `Starting`, `Stopping`, `Sleeping`, `Crashed`. `start` and `stop` run under the host timeout. `ServerState` lives in `infrarust_plugin_sdk::services`.

```rust
use infrarust_plugin_sdk::services::ServerState;

let servers = ctx.server_manager();
if servers.state("survival") == Some(ServerState::Sleeping) {
    servers.start("survival")?;
}
```

## Bans

`ctx.ban_service()` returns `Bans`. Every method needs the opt-in `ban` capability and runs under the host timeout. `BanTarget` and `BanEntry` live in `infrarust_plugin_sdk::services`. A `BanTarget` is one of three variants:

```rust
pub enum BanTarget {
    Ip(String),
    Username(String),
    Uuid(String),
}
```

```rust
pub fn ban(&self, target: &BanTarget, reason: Option<&str>, duration_ms: Option<u64>)
    -> Result<(), ServiceError>;
pub fn unban(&self, target: &BanTarget) -> Result<bool, ServiceError>;
pub fn is_banned(&self, target: &BanTarget) -> Result<bool, ServiceError>;
pub fn get(&self, target: &BanTarget) -> Result<Option<BanEntry>, ServiceError>;
pub fn all(&self) -> Result<Vec<BanEntry>, ServiceError>;
```

`duration_ms` of `None` is a permanent ban. A `BanEntry` has these fields:

| Field | Type | Meaning |
| --- | --- | --- |
| `target` | `BanTarget` | Who is banned |
| `reason` | `Option<String>` | Optional reason text |
| `expires_at` | `Option<u64>` | Epoch millis, `None` for permanent |
| `created_at` | `u64` | When the ban was issued, epoch millis |
| `source` | `String` | What issued the ban |

```rust
use infrarust_plugin_sdk::services::BanTarget;

let bans = ctx.ban_service();
let target = BanTarget::Username("Griefer".into());
bans.ban(&target, Some("griefing"), Some(86_400_000))?; // 24h
```

## Config

`ctx.config_service()` returns `Config`. Reads are baseline (`config-read`).

```rust
pub fn get(&self, key: &str) -> Option<String>;
pub fn server(&self, server: &str) -> Option<ServerConfig>;
pub fn servers(&self) -> Vec<ServerConfig>;
```

`get` reads a single string value by key. `server` and `servers` return `ServerConfig` records with the proxy's view of each backend (id, addresses, domains, proxy mode, limbo handlers, max players, and more).

```rust
let greeting = ctx.config_service()
    .get("greeting")
    .unwrap_or_else(|| "Welcome".to_string());
```

## Commands and codec filters

Two more host services register guest callbacks rather than reading state, and each has its own page. Command registration is baseline (`command`): call `ctx.command(name, handler)` to get a `CommandBuilder`, chain `alias`, `aliases`, `description`, and `completer`, then `register()`. Codec-filter registration is the opt-in `codec-filter` capability and goes through the `Plugin::register_codec_filters(reg: &mut CodecRegistrar)` method, where `reg.add(id, priority, constructor)` declares one filter. See [Commands](./commands) and [Codec filters](./codec-filters) for the full contracts.

## Scheduler

Scheduling is baseline (`scheduler`). The methods are on `Context` and return a `TaskHandle` (a `u64`).

```rust
pub fn delay(&self, after: Duration, task: impl FnMut() + 'static) -> TaskHandle;
pub fn interval(&self, period: Duration, task: impl FnMut() + 'static) -> TaskHandle;
pub fn cancel(&self, handle: TaskHandle);
```

`delay` runs the closure once after the duration. `interval` runs it repeatedly. Both fire through the host's `on-scheduled-task` dispatch back into the plugin.

```rust
use std::time::Duration;

let handle = ctx.interval(Duration::from_secs(60), || {
    info!("{} players online", Players.online_count());
});
// later, stop it:
ctx.cancel(handle);
```

## Logging

The log interface is always linked, so the logging macros work without a capability. Each takes a `format!`-style argument and writes to the host's tracing output tagged with your plugin id.

```rust
trace!("inbound packet {id}");
debug!("state = {state:?}");
info!("{name} connected");
warn!("retrying {attempt}/{max}");
error!("failed: {err}");
```

## Building chat components

`Component` builds the text-component JSON the host expects for messages, titles, and action bars. Chain styling, append children, then serialize with `into_json` (or `From<Component> for String`) and pass the JSON to a host method.

```rust
let msg = Component::text("Server: ")
    .color("gray")
    .append(Component::text("survival").color("green").bold());

player.send_message(&msg.into_json())?;
```

| Method | Effect |
| --- | --- |
| `Component::text(impl Into<String>)` | Start a component with text |
| `.color(impl Into<String>)` | Set the color |
| `.bold()` | Mark bold |
| `.italic()` | Mark italic |
| `.append(Component)` | Add a child to `extra` |
| `.into_json()` | Serialize to the component JSON string |

## Errors

Two error types surface from host services.

```rust
pub enum PlayerError {
    NotActive,
    Disconnected,
    SendFailed(String),
    ServerNotFound(String),
    SwitchFailed(String),
}

pub enum ServiceError {
    NotFound(String),
    OperationFailed(String),
    Unavailable(String),
}
```

`PlayerError` comes from `Player` actions. `ServiceError` comes from `Servers` and `Bans`; a timed-out call yields `ServiceError::Unavailable`.

## A full example

The `host-caller` fixture reads two baseline services in `on_enable`:

```rust
use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct HostCaller;

#[plugin(id = "host-caller", name = "Host Caller Fixture")]
impl Plugin for HostCaller {
    fn on_enable(&self, _ctx: &Context) -> Result<(), String> {
        std::fs::write("count.txt", Players.online_count().to_string().as_bytes())
            .map_err(|e| e.to_string())?;
        if let Some(greeting) = Config.get("greeting") {
            std::fs::write("greeting.txt", greeting.as_bytes()).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
```

## See also

- [Capabilities](./capabilities): the full capability list and how grants work
- [Events](./events): react to player and server lifecycle changes
- [Commands](./commands): register commands that call these services
- [Codec filters](./codec-filters): register codec filters and the per-call budget
- [Limbo](./limbo): hold players in a virtual world from a handler
- [API reference](./api-reference): the complete SDK surface
- [Examples](./examples): runnable plugins
- [Configuration](../../configuration/): proxy config and the `[plugins]` table
