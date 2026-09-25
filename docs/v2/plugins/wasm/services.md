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
Calls into `server-manage` and `ban` wait for the proxy's answer for at most `host_call_timeout` (30 s by default), and return early enough to leave the calling handler time to act before its own deadline. When a limit runs out, the call returns `ServiceError::Unavailable` and your code carries on. See [Slow services and deadlines](#slow-services-and-deadlines).
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
`disconnect` is dispatched in the background on the host, so it returns immediately. `switch_server` hands the request to the player's session and waits at most 250 ms for the session to take it, less when the handler's deadline is closer; if it can't, it returns `PlayerError::SwitchFailed`. `send_packet` without the `raw-packet` capability returns `PlayerError::SendFailed("missing capability: raw-packet")` rather than trapping.
:::

## Servers

`ctx.server_manager()` returns `Servers`. Every method needs the opt-in `server-manage` capability.

```rust
pub fn state(&self, server: &str) -> Option<ServerState>;
pub fn start(&self, server: &str) -> Result<(), ServiceError>;
pub fn stop(&self, server: &str) -> Result<(), ServiceError>;
pub fn all(&self) -> Vec<(String, ServerState)>;
```

`ServerState` is one of `Online`, `Offline`, `Starting`, `Stopping`, `Sleeping`, `Crashed`. `start` and `stop` run under the host timeout and the handler's deadline, see [Slow services and deadlines](#slow-services-and-deadlines). `ServerState` lives in `infrarust_plugin_sdk::services`.

```rust
use infrarust_plugin_sdk::services::ServerState;

let servers = ctx.server_manager();
if servers.state("survival") == Some(ServerState::Sleeping) {
    servers.start("survival")?;
}
```

## Bans

`ctx.ban_service()` returns `Bans`. Every method needs the opt-in `ban` capability and runs under the host timeout and the handler's deadline, see [Slow services and deadlines](#slow-services-and-deadlines). `BanTarget` and `BanEntry` live in `infrarust_plugin_sdk::services`. A `BanTarget` is one of three variants:

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

`duration_ms` of `None` is a permanent ban. `BanTarget::Ip` also accepts a CIDR range such as `"203.0.113.0/24"`, which bans every address inside it. The calls go to whichever ban provider the operator selected, and a ban your plugin issues is attributed to it (`source` reads `plugin:<your id>`). With `[ban] provider = "none"` every call returns `service-error::unavailable`. A `BanEntry` has these fields:

| Field | Type | Meaning |
| --- | --- | --- |
| `target` | `BanTarget` | Who is banned |
| `reason` | `Option<String>` | Optional reason text |
| `expires_at` | `Option<u64>` | Epoch millis, `None` for permanent |
| `created_at` | `u64` | When the ban was issued, epoch millis |
| `source` | `String` | Who issued the ban: `console`, `web-api`, `plugin:<id>`, `player:<name>` or `system` |

```rust
use infrarust_plugin_sdk::services::BanTarget;

let bans = ctx.ban_service();
let target = BanTarget::Username("Griefer".into());
bans.ban(&target, Some("griefing"), Some(86_400_000))?; // 24h
```

## Slow services and deadlines

`start`, `stop` and every `Bans` method wait for the proxy's answer, and that answer can be slow: a ban list kept in a remote database, a server that takes a while to boot. The host bounds each of these calls so that a slow service becomes an error your code can act on, rather than an answer that arrives after the proxy stopped listening.

Every call into your plugin carries a deadline, set when the proxy makes the call:

| Call into the plugin | Deadline |
| --- | --- |
| Event handler | `[events] handler_timeout` (10 s by default): how long the event bus waits for a listener |
| Command, tab completion, scheduled task, limbo callback | `max_call_duration` in `[wasm]` (60 s by default): nothing in the proxy stops waiting for these earlier |
| `on_enable`, `on_disable` | none |

A service call made during that call returns at the earliest of:

- the service's answer;
- `host_call_timeout` after the service call started (`[wasm]`, 30 s by default);
- a margin before the deadline. The margin is a fifth of the deadline, capped at 250 ms, and leaves your code time to decide after the error. With the default 10 s `handler_timeout`, a ban check inside an event handler returns by 9.75 s.

On expiry the call returns `ServiceError::Unavailable`. The message is `host call timed out` when `host_call_timeout` ran out, and `host call timed out: the plugin call is close to its deadline` when the deadline was the limit. `switch_server` follows the same rule with its own 250 ms limit and returns `PlayerError::SwitchFailed`.

Because the error arrives before the event bus gives up on the handler, whatever the handler decides is applied to the event. Choose on purpose: a ban check that denies on error keeps banned players out while the ban store is down (fail closed), one that ignores the error lets everyone in (fail open).

```rust
ctx.on(EventPriority::Early, |event: &mut PreLoginEvent| {
    let target = BanTarget::Username(event.profile.username.clone());
    match Bans.is_banned(&target) {
        Ok(false) => {}
        Ok(true) => event.deny("You are banned"),
        Err(_) => event.deny("The ban check is unavailable, try again in a moment"),
    }
});
```

A call still waiting in the plugin's queue when its deadline passes is dropped without running: its caller could no longer use the result. See [Lifecycle](./lifecycle#deadlines).

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
pub fn delay(&self, after: Duration, task: impl FnOnce() + 'static) -> TaskHandle;
pub fn interval(&self, period: Duration, task: impl FnMut() + 'static) -> TaskHandle;
pub fn cancel(&self, handle: TaskHandle);
```

`delay` runs the closure once after the duration and drops it right after. `interval` runs it repeatedly. Both fire through the host's `on-scheduled-task` dispatch back into the plugin. `cancel` stops the task on the host and drops the closure in the guest; calling it from inside the task's own callback is fine, and the closure is dropped once that call returns.

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
