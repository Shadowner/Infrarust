---
title: Host Services
description: Query and act on players by id, manage servers and bans, read config, schedule tasks, log, build text components, and handle host errors from a WASM plugin.
outline: [2, 3]
---

# Host Services

A WASM plugin reaches the proxy through host services. Each service is a set of associated functions on a unit struct (`Players`, `Servers`, `Bans`, `Config`, `LoadBalancer`, `Messaging`, `Proxy`, `Plugins`) or a method on the [`Context`](./api-reference#the-guest-export) you receive in `on_enable`, and each one is gated by a [capability](./capabilities). Baseline capabilities are granted to every plugin; opt-in capabilities must be listed in the plugin's TOML `permissions`.

```rust
use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct MyPlugin;

#[plugin(id = "my-plugin", name = "My Plugin")]
impl Plugin for MyPlugin {
    fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> {
        let online = Players::count(); // [!code focus]
        info!("{online} players online"); // [!code focus]
        Ok(())
    }
}
```

The service functions need no handle, so you can call them from a command, a scheduled task, an event handler or a limbo callback just as well.

## Capabilities at a glance

| Service | Entry point | Capability | Grant |
| --- | --- | --- | --- |
| Player lookups | `Players::get`, `by_name`, `by_uuid`, `list`, `on_server`, `count` | `player-read` | baseline |
| Player permission check | `Player::has_permission` | `player-read` | baseline |
| Player actions | `Player::send_message`, `send_title`, `clear_title`, `send_action_bar`, `set_player_list_header_footer`, `show_boss_bar`, `send_resource_pack`, `remove_resource_pack`, `disconnect`, `switch_server`, `connect`, `transfer`, `store_cookie`, `request_cookie`, `refresh_permissions` | `player-write` | baseline |
| `Player::send_packet` | on the `Player` handle | `raw-packet` | opt-in |
| Servers | `Servers::*` | `server-manage` | opt-in |
| Bans | `Bans::*` | `ban` | opt-in |
| Ban provider | `ctx.provide_bans` | `ban-provider` | opt-in |
| Permission provider | `ctx.provide_permissions` | `permission-provider` | opt-in |
| Permission snapshots | `Permissions::set_snapshot`, `release` | `permission-provider` | opt-in |
| Config reads | `Config::get`, `server`, `servers`, `server_document`, `server_sources`, `proxy_document`, `effective_proxy_document` | `config-read` | baseline |
| Config write | `Config::write_proxy_document` | `config-write` | opt-in |
| Load balancer reads | `LoadBalancer::strategy`, `backends` | `config-read` | baseline |
| Load balancer maintenance | `LoadBalancer::set_drained`, `reset_backend` | `server-manage` | opt-in |
| Plugin messaging | `Messaging::*` | `plugin-messaging` | opt-in |
| Named events | `ctx.on_named`, `ctx.fire_named` | `event-bus` | baseline |
| Proxy information | `Proxy::details`, `version`, `granted_capabilities`, `has_capability` | always available | always |
| Plugin registry | `Plugins::list`, `get`, `is_loaded` | always available | always |
| Commands | `ctx.command(name)` | `command` | baseline |
| Codec filters | `Plugin::register_codec_filters` | `codec-filter` | opt-in |
| Scheduler | `ctx.delay` / `ctx.interval` / `ctx.interval_with_delay` | `scheduler` | baseline |
| Text | `Component::from_json`, `from_legacy`, `to_json` | always available | always |
| Logging | `info!` family | always available | always |

Opt-in capabilities are listed in the plugin's config block. Capability strings are kebab-case.

```toml
[plugins.my-plugin]
permissions = ["server-manage", "ban", "raw-packet"]
```

## Errors

Every host call that can fail returns `Result<T, Error>`. An `Error` has a `kind` to branch on and a `message` for logs:

```rust
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

pub enum ErrorKind {
    InvalidArgument,
    NotFound,
    PermissionDenied,
    Unavailable,
    Timeout,
    PlayerGone,
    Conflict,
    InvalidState,
    Unsupported,
    Internal,
}
```

`Error` implements `std::error::Error`, and `?` turns it into the `PluginError` that `on_enable` and `on_disable` return. A call the plugin lacks the capability for returns `PermissionDenied` with a message that names it, for example `missing capability: ban`. The host also logs that refusal, at most once a minute per capability. See [the WIT error kinds](./api-reference#errors) for when each kind is raised.

```rust
match Bans::is_banned(&BanTarget::Username("Griefer".into())) {
    Ok(banned) => info!("banned: {banned}"),
    Err(e) if e.kind() == ErrorKind::PermissionDenied => warn!("grant `ban` to use bans"),
    Err(e) => error!("ban check failed: {e}"),
}
```

## Players

Players are addressed by `PlayerId`. The lookups return a `PlayerInfo` snapshot; the actions go through a `Player` handle, which is just the id.

```rust
impl Players {
    pub fn get(id: PlayerId) -> Option<PlayerInfo>;
    pub fn by_name(username: &str) -> Option<PlayerInfo>;
    pub fn by_uuid(uuid: Uuid) -> Option<PlayerInfo>;
    pub fn list() -> Vec<PlayerInfo>;
    pub fn on_server(server: &ServerId) -> Vec<PlayerInfo>;
    pub fn count() -> u32;
    pub fn count_on(server: &ServerId) -> u32;
}
```

The lookups are the contract's infallible reads: without `player-read` they answer `None`, an empty list or `0`.

`PlayerInfo` carries what the proxy knows about the player when you asked:

| Field | Type | Meaning |
| --- | --- | --- |
| `player` | `PlayerRef` | `id`, `uuid` and `username` |
| `profile` | `GameProfile` | UUID, username, properties |
| `protocol` | `i32` | Client protocol version |
| `remote_addr` | `SocketAddr` | Client socket address |
| `current_server` | `Option<ServerId>` | Backend the player is on |
| `online_mode` | `bool` | Authenticated against Mojang |
| `connected` | `bool` | TCP connection is live |
| `active` | `bool` | Player is in an actionable state |
| `connected_at` | `SystemTime` | When the player connected |
| `virtual_host` | `Option<String>` | The address the client connected to |
| `client_brand` | `Option<String>` | The client brand, once the client sent it |
| `ping` | `Option<Duration>` | Latest round-trip time |
| `settings` | `Option<ClientSettings>` | Locale, view distance, chat mode, skin parts, main hand, filtering, listing, particles, once the client sent them |
| `known_channels` | `Vec<String>` | Plugin channels the client registered |

### The Player handle

Get one with `PlayerInfo::handle()`, `PlayerRef::handle()` (every player-scoped event carries a `PlayerRef`), or `Player::new(id)`. The handle is `Copy`; it stays valid as a value after the player leaves, and calls on it then return `PlayerGone`.

```rust
ctx.on::<PostLoginEvent>(EventPriority::Normal, |event| {
    let player = event.player.handle();
    let _ = player.send_message(Component::text("Welcome").color(NamedColor::Gold));
    let _ = player.send_title(&TitleData::new("Hello", "have fun").stay(40));
})?;
```

| Action | Signature | Capability |
| --- | --- | --- |
| `info` | `fn info(&self) -> Option<PlayerInfo>` | `player-read` |
| `has_permission` | `fn has_permission(&self, node: &str) -> Result<bool, Error>` | `player-read` |
| `send_message` | `fn send_message(&self, message: impl Into<Component>) -> Result<(), Error>` | `player-write` |
| `send_title` | `fn send_title(&self, title: &TitleData) -> Result<(), Error>` | `player-write` |
| `send_action_bar` | `fn send_action_bar(&self, message: impl Into<Component>) -> Result<(), Error>` | `player-write` |
| `disconnect` | `fn disconnect(&self, reason: impl Into<Component>) -> Result<(), Error>` | `player-write` |
| `switch_server` | `fn switch_server(&self, server: impl Into<ServerId>) -> Result<(), Error>` | `player-write` |
| `connect` | `fn connect(&self, server: impl Into<ServerId>) -> Result<ConnectionResult, Error>` | `player-write` |
| `set_player_list_header_footer` | `fn set_player_list_header_footer(&self, header: impl Into<Component>, footer: impl Into<Component>) -> Result<(), Error>` | `player-write` |
| `clear_title`, `reset_title` | `fn clear_title(&self) -> Result<(), Error>` | `player-write` |
| `show_boss_bar` | `fn show_boss_bar(&self, bar: &BossBar) -> Result<BossBarHandle, Error>` | `player-write` |
| `send_resource_pack` | `fn send_resource_pack(&self, pack: &ResourcePackRequest) -> Result<(), Error>` | `player-write` |
| `remove_resource_pack` | `fn remove_resource_pack(&self, id: Option<Uuid>) -> Result<(), Error>` | `player-write` |
| `transfer` | `fn transfer(&self, host: impl Into<String>, port: u16) -> Result<(), Error>` | `player-write` |
| `store_cookie` | `fn store_cookie(&self, key: &str, data: &[u8]) -> Result<(), Error>` | `player-write` |
| `request_cookie` | `fn request_cookie(&self, key: &str) -> Result<Option<Vec<u8>>, Error>` | `player-write` |
| `refresh_permissions` | `fn refresh_permissions(&self) -> Result<(), Error>` | `player-write` |
| `send_packet` | `fn send_packet(&self, packet_id: i32, data: Vec<u8>) -> Result<(), Error>` | `raw-packet` |

:::info
`disconnect` queues the kick and returns at once. `switch_server` hands the request to the player's session and waits at most 250 ms for the session to take it, less when the calling handler's deadline is closer; if it can't, it returns `Timeout`. `connect` waits for the outcome of the switch instead and answers a `ConnectionResult` (`Success`, `AlreadyConnected`, `Denied(reason)`, `Failed(reason)` or `Cancelled`); `transfer`, `request_cookie` and `refresh_permissions` also wait for their answer. All four follow [Slow services and deadlines](#slow-services-and-deadlines). `has_permission` answers after the permission provider and the node's default.
:::

`connect` runs the whole switch on the player's session, which fires `ServerPreConnectEvent` and the other connection events. A plugin that listens to one of those events cannot answer it while it is itself waiting in `connect`: the event bus gives up on that listener after `[events] handler_timeout` and the switch goes on without it. A plugin that listens to connection events should move players with `switch_server`, which does not wait for the switch.

From a handler the player's own session is waiting on, such as that player's `ChatMessageEvent` or a limbo callback for that player, `connect` and `request_cookie` for that player return `InvalidState` at once: the session could not answer before the handler returns. A command handler is not in that case and can wait for its own player's switch. See [Calls that wait on the player's own session](./threading#calls-that-wait-on-the-player-s-own-session).

`transfer` needs a 1.20.5+ client and fires `PreTransferEvent` first; an older client answers `Unsupported`. The plugin that calls `transfer` does not get a say in its own `PreTransferEvent`: see [Firing an event you listen to](./events#named-events). Cookie keys are `namespace:path` (a bare path means `minecraft:`), and a cookie holds at most 5120 bytes. A resource pack carries its own `Uuid` so you can remove it later; `hash` must be the 40 hexadecimal characters of the pack's SHA-1, and a bad one returns `InvalidArgument`.

A boss bar stays until you hide it or the player leaves. `show_boss_bar` returns a `BossBarHandle` you keep to change it:

```rust
let bar = player.show_boss_bar(
    &BossBar::new("Restart in 5 minutes")
        .color(BossBarColor::Red)
        .overlay(BossBarOverlay::Notched10),
)?;
bar.set_progress(0.5)?;
bar.set_title("Restart in 2 minutes")?;
bar.hide()?;
```

`set_title`, `set_progress`, `set_style(color, overlay)` and `set_flags` update it, `hide` removes it; a bar the plugin already hid answers `NotFound`. A plugin shows at most 256 bars at once, a further one returns `Conflict`, and the bars of a plugin instance that is unloaded or recovered are hidden with it.

## Servers

Every `Servers` function needs the opt-in `server-manage` capability.

```rust
impl Servers {
    pub fn state(server: &ServerId) -> Result<Option<ServerState>, Error>;
    pub fn start(server: &ServerId) -> Result<(), Error>;
    pub fn stop(server: &ServerId) -> Result<(), Error>;
    pub fn list() -> Result<Vec<ServerStatus>, Error>;
}
```

`ServerState` is one of `Online`, `Offline`, `Starting`, `Stopping`, `Sleeping`, `Crashed`. `ServerStatus` pairs a `server` with its `state`. `start` and `stop` run under the host timeout and the handler's deadline, see [Slow services and deadlines](#slow-services-and-deadlines).

```rust
let survival = ServerId::from("survival");
if Servers::state(&survival)? == Some(ServerState::Sleeping) {
    Servers::start(&survival)?;
}
```

## Bans

Every `Bans` function needs the opt-in `ban` capability and runs under the host timeout and the handler's deadline, see [Slow services and deadlines](#slow-services-and-deadlines).

```rust
pub enum BanTarget {
    Ip(IpAddr),
    IpRange(String),
    Username(String),
    Uuid(Uuid),
}

impl Bans {
    pub fn ban(request: BanRequest) -> Result<BanEntry, Error>;
    pub fn unban(target: &BanTarget) -> Result<Option<BanEntry>, Error>;
    pub fn get(target: &BanTarget) -> Result<Option<BanEntry>, Error>;
    pub fn is_banned(target: &BanTarget) -> Result<bool, Error>;
    pub fn list(cursor: Option<&str>, limit: u32) -> Result<BanPage, Error>;
}
```

`BanRequest::new(target)` builds a request that kicks the target and announces the ban; chain `reason`, `duration`, `kick(false)` or `silent(true)`. A request without a duration is permanent. `IpRange` takes CIDR notation such as `"203.0.113.0/24"` and bans every address inside it; a range that does not parse returns `InvalidArgument`. `unban` answers the entry it removed, `list` pages through the bans with the `next_cursor` of the previous page.

The calls go to whichever ban provider the operator selected, and a ban your plugin issues is attributed to it (`source` reads `plugin:<your id>`). With `[ban] provider = "none"` every call returns `Unavailable`. A `BanEntry` has these fields:

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | `String` | The provider's id for the ban |
| `target` | `BanTarget` | Who is banned |
| `reason` | `Option<String>` | Optional reason text |
| `source` | `String` | Who issued the ban: `console`, `web-api`, `plugin:<id>`, `player:<name>` or `system` |
| `created_at` | `SystemTime` | When the ban was issued |
| `expires_at` | `Option<SystemTime>` | `None` for permanent |

```rust
use std::time::Duration;

let target = BanTarget::Username("Griefer".into());
Bans::ban(BanRequest::new(target).reason("griefing").duration(Duration::from_secs(86_400)))?;
```

A plugin can also be the ban provider itself, the one these calls reach; see [Bans](./bans). While it is, its own `Bans` calls answer `Unavailable` at once, since the host cannot call back into the instance that is making the call.

## Slow services and deadlines

`start`, `stop`, every `Bans` function, `Player::connect`, `transfer`, `request_cookie`, `refresh_permissions`, `ctx.fire_named`, `Permissions::set_snapshot` and `Permissions::release` wait for the proxy's answer, and that answer can be slow: a ban list kept in a remote database, a server that takes a while to boot. The host bounds each of these calls so that a slow service becomes an error your code can act on, rather than an answer that arrives after the proxy stopped listening.

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

On expiry the call returns an `Error` of kind `Timeout`. The message is `host call timed out` when `host_call_timeout` ran out, and `host call timed out: the plugin call is close to its deadline` when the deadline was the limit. `switch_server` follows the same rule with its own 250 ms limit.

Because the error arrives before the event bus gives up on the handler, whatever the handler decides is applied to the event. Choose on purpose: a ban check that denies on error keeps banned players out while the ban store is down (fail closed), one that ignores the error lets everyone in (fail open).

```rust
ctx.on::<PreLoginEvent>(EventPriority::Early, |event| {
    let target = BanTarget::Username(event.profile.username.clone());
    match Bans::is_banned(&target) {
        Ok(false) => {}
        Ok(true) => event.deny("You are banned"),
        Err(_) => event.deny("The ban check is unavailable, try again in a moment"),
    }
})?;
```

A call still waiting in the plugin's queue when its deadline passes is dropped without running: its caller could no longer use the result. See [Lifecycle](./lifecycle#deadlines).

## Config

Reads need the baseline `config-read` capability.

```rust
impl Config {
    pub fn get(key: &str) -> Result<Option<String>, Error>;
    pub fn server(server: &ServerId) -> Result<Option<ServerConfig>, Error>;
    pub fn servers() -> Result<Vec<ServerConfig>, Error>;
}
```

`get` reads one value of the configuration the proxy runs on by its dotted path, as the native `ConfigService::get_value` does: a string comes back as its text, a number or a boolean as its `to_string()`, a table or an array as inline TOML, a secret field as `<redacted>`, and a path with nothing at it as `None`. `server` and `servers` return `ServerConfig` records with the proxy's view of each backend (id, network, addresses, domains, proxy mode, limbo handlers, max players, disconnect message, proxy protocol, server manager).

```rust
let retries: u32 = Config::get("keepalive.retries")?
    .and_then(|value| value.parse().ok())
    .unwrap_or(3);
```

The documents mirror the native `ConfigService`, with every secret field redacted:

```rust
impl Config {
    pub fn server_document(server: &ServerId) -> Result<Option<String>, Error>;
    pub fn server_sources() -> Result<Vec<ServerSource>, Error>;
    pub fn proxy_document() -> Result<String, Error>;
    pub fn effective_proxy_document() -> Result<String, Error>;
    pub fn write_proxy_document(document: &str) -> Result<(), Error>;
}
```

`server_document` is the full TOML of one server, whatever provider supplied it. `server_sources` says where each server came from (`provider_id`, `provider_type`) and whether a plugin may rewrite it (`editable`). `proxy_document` is the global configuration file as written, `effective_proxy_document` the configuration the proxy runs on, with CLI overrides and defaults applied.

`write_proxy_document` replaces the configuration file and needs the opt-in `config-write` capability. Secret fields the document leaves out or carries redacted keep their value on disk, so a document read with `proxy_document` can be edited and written back. The file is validated first: a document that does not parse or that the proxy refuses returns `InvalidArgument`, a failed write `Unavailable`. Nothing changes in the running proxy; the new file applies on restart.

## Load balancer

`LoadBalancer` mirrors the native `LoadBalancerService`. The reads need `config-read`, the maintenance calls the opt-in `server-manage`.

```rust
impl LoadBalancer {
    pub fn strategy(server: &ServerId) -> Result<Option<String>, Error>;
    pub fn backends(server: &ServerId) -> Result<Vec<BackendStatus>, Error>;
    pub fn set_drained(server: &ServerId, address: &ServerAddress, drained: bool) -> Result<(), Error>;
    pub fn reset_backend(server: &ServerId, address: &ServerAddress) -> Result<(), Error>;
}
```

A `BackendStatus` has the `address`, its configured `weight` and the `effective_weight` after slow start, its `state` (`Healthy`, `Probing`, `Unhealthy`, `Draining`), `active_connections`, `healthy_since`, `ejections` and `last_failure_ago`. Draining never closes an established session; `reset_backend` clears the failure history of an address. An unknown server or address returns `NotFound`.

## Plugin messaging

`Messaging` needs the opt-in `plugin-messaging` capability. A `ChannelId` names a channel by its modern id (`namespace:name`), its legacy name (before 1.13), or both; the host validates it and returns `InvalidArgument` for a malformed one.

```rust
impl Messaging {
    pub fn register(channel: &ChannelId) -> Result<(), Error>;
    pub fn unregister(channel: &ChannelId) -> Result<bool, Error>;
    pub fn channels() -> Result<Vec<ChannelId>, Error>;
    pub fn send_to_player(player: PlayerId, channel: &ChannelId, data: &[u8]) -> Result<(), Error>;
    pub fn send_to_backend(player: PlayerId, channel: &ChannelId, data: &[u8]) -> Result<(), Error>;
    pub fn send_to_server(server: &ServerId, channel: &ChannelId, data: &[u8]) -> Result<u32, Error>;
}
```

Registering a channel is what makes the proxy fire [`PluginMessageEvent`](./events#plugin-messages) for it. `send_to_player` sends to the client, `send_to_backend` to the backend that player is on, and `send_to_server` through any player connected to that server; it answers how many players could carry the message and returns `Unavailable` when none is there. A message to a backend is limited to 32767 bytes, one to a client to 1 MiB.

```rust
let channel = ChannelId::modern("myplugin:sync");
Messaging::register(&channel)?;
Messaging::send_to_server(&ServerId::from("lobby"), &channel, b"refresh")?;
```

## Proxy information and plugins

`Proxy` and `Plugins` are always available and read-only.

```rust
impl Proxy {
    pub fn details() -> ProxyDetails;
    pub fn version() -> String;
    pub fn granted_capabilities() -> Vec<Capability>;
    pub fn has_capability(capability: Capability) -> bool;
}

impl Plugins {
    pub fn list() -> Vec<PluginInfo>;
    pub fn get(id: &str) -> Option<PluginInfo>;
    pub fn is_loaded(id: &str) -> bool;
}
```

`ProxyDetails` mirrors the native `ProxyInfo`: the version, the bind address, the connection limits and timeouts, the rate limits, the status cache and keepalive settings, which optional features are on, and what happens to an unknown domain. `granted_capabilities` lists what this plugin was granted, so a plugin can switch a feature off instead of failing a call. A `PluginInfo` has the `id`, `name`, `version`, `authors`, `description`, `state` and `dependencies` of each loaded plugin. The list holds the plugins enabled right now, each with the state `enabled`: a plugin appears as soon as its `on_enable` returns, so `on_enable` sees every plugin enabled before this one, and it leaves the list when its disabling starts.

```rust
if Proxy::has_capability(Capability::Ban) && Plugins::is_loaded("auth") {
    info!("running on Infrarust {}", Proxy::version());
}
```

## Commands and codec filters

Two more host services register guest callbacks rather than reading state, and each has its own page. Command registration is baseline (`command`): `ctx.command(name)` returns a builder; chain `aliases`, `description`, `usage`, `permission`, `hidden`, `handler` and `completer`, then `register()`, which answers what the host registered. Codec-filter registration is the opt-in `codec-filter` capability and goes through `Plugin::register_codec_filters(reg: &mut CodecRegistrar)`, where `reg.add(id, priority, constructor)` declares one filter. See [Commands](./commands) and [Codec filters](./codec-filters).

## Scheduler

Scheduling is baseline (`scheduler`). The methods are on `Context` and return a `TaskHandle`.

```rust
pub fn delay(&self, after: Duration, task: impl FnOnce() + 'static) -> Result<TaskHandle, Error>;
pub fn interval(&self, period: Duration, task: impl FnMut() + 'static) -> Result<TaskHandle, Error>;
pub fn interval_with_delay(
    &self,
    period: Duration,
    initial_delay: Duration,
    task: impl FnMut() + 'static,
) -> Result<TaskHandle, Error>;
pub fn cancel(&self, handle: TaskHandle);
```

`delay` runs the closure once after the duration and drops it right after. `interval` runs it every period, the first time one period from now; `interval_with_delay` sets the first run separately. Both fire through the host's `on-scheduled-task` dispatch back into the plugin. `cancel` (or `TaskHandle::cancel`) stops the task on the host and drops the closure in the guest; calling it from inside the task's own callback is fine, and the closure is dropped once that call returns.

```rust
use std::time::Duration;

let handle = ctx.interval(Duration::from_secs(60), || {
    info!("{} players online", Players::count());
})?;
handle.cancel();
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

## Text components

`Component` is the SDK's text component, at parity with the native one: text, translatable and keybind content, named and hex colors, every decoration, font, insertion, shadow color, click and hover events, and children. It crosses the boundary as a validated arena the SDK builds for you.

```rust
let message = Component::text("Server: ")
    .color(NamedColor::Gray)
    .append(
        Component::text("survival")
            .color("#55ff55")
            .bold()
            .click(ClickEvent::RunCommand("/server survival".into()))
            .hover(HoverEvent::show_text("Click to join")),
    );
player.send_message(message)?;
```

| Builder | Effect |
| --- | --- |
| `Component::text(s)`, `translatable(key)`, `translatable_with(key, args)`, `keybind(key)` | Start a component |
| `.fallback(text)` | Fallback text for a translatable component |
| `.color(c)` | A `NamedColor`, a `TextColor`, or a string such as `"gold"` or `"#ff8800"`; an unknown string leaves the color unset |
| `.bold()`, `.italic()`, `.underlined()`, `.strikethrough()`, `.obfuscated()`, `.decoration(d, value)` | Decorations |
| `.font(id)`, `.insertion(text)`, `.shadow_color(argb)` | Other style |
| `.click(ClickEvent)`, `.hover(HoverEvent)` | Events |
| `.append(child)` | Add a child |
| `.to_plain()` | The text without styling, computed in the guest |

A `&str` or `String` converts into a plain-text component, never into JSON. To use JSON or legacy `&` codes, parse them with the host, which uses the proxy's own parser:

```rust
let motd = Component::from_json(r#"{"text":"Hello","color":"gold"}"#)?;
let legacy = Component::from_legacy("&aGreen &lbold");
let json: String = motd.to_json()?;
```

A component deeper than 64 levels, larger than 4096 nodes or 256 KiB of text is refused by the host with `InvalidArgument`.

## A full example

The `host-caller` test fixture reads two baseline services in `on_enable`:

```rust
use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct HostCaller;

#[plugin(id = "host-caller", name = "Host Caller Fixture")]
impl Plugin for HostCaller {
    fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> {
        std::fs::write("count.txt", Players::count().to_string())?;
        if let Ok(Some(greeting)) = Config::get("greeting") {
            std::fs::write("greeting.txt", greeting)?;
        }
        Ok(())
    }
}
```

The fixture runs against a mock config service that has a `greeting` key. Against a real proxy, `Config::get` takes a dotted path of the running proxy config, such as `bind` or `keepalive.retries`.

## See also

- [Capabilities](./capabilities): the full capability list and how grants work
- [Events](./events): react to player and server lifecycle changes
- [Commands](./commands): register commands that call these services
- [Codec filters](./codec-filters): register codec filters and the per-call budget
- [Limbo](./limbo): hold players in a virtual world from a handler
- [API reference](./api-reference): the WIT contract behind these wrappers
- [Examples](./examples): runnable plugins
- [Configuration](../../configuration/): proxy config and the `[plugins]` table
