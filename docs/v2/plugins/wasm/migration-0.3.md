---
title: Migrating to 0.3
description: Port a WASM plugin from the infrarust:plugin@0.2.3 contract to 0.3.0, change by change.
outline: [2, 3]
---

# Migrating to 0.3

`infrarust:plugin@0.3.0` replaces the 0.2.3 contract. The new contract mirrors the native API: players are addressed by id, text travels as a real component tree instead of JSON strings, every host call that can fail has an error channel, and resulted events show the current result so a handler can build on what earlier handlers decided.

The two contracts are not compatible. A proxy on 0.3 refuses a component built for 0.2.3 at discovery, before running any of its code:

```text
plugin built for infrarust:plugin@0.2.3; this host supports infrarust:plugin@0.3.x, rebuild it with an infrarust-plugin-sdk that targets infrarust:plugin@0.3.x
```

Update the `infrarust-plugin-sdk` dependency, rebuild, and fix the compile errors with the tables below. Most plugins need a handful of mechanical changes.

## The plugin trait

| 0.2.3 | 0.3.0 |
|-------|-------|
| `fn on_enable(&self, ctx: &Context) -> Result<(), String>` | `fn on_enable(&self, ctx: &Context) -> Result<(), PluginError>` |
| `fn on_disable(&self, ctx: &Context) -> Result<(), String>` | `fn on_disable(&self, ctx: &Context) -> Result<(), PluginError>` |
| `.map_err(\|e\| e.to_string())?` | `?` works on SDK errors, `std::io::Error`, `String` and `&str` |
| `return Err(format!(...))` | `return Err(format!(...).into())` |
| no reason | `ctx.enable_reason()` is `Initial` or `Recovered(RecoveryInfo { attempt, cause })`; `ctx.disable_reason()` is `Shutdown`, `Unload` or `Quarantine` |
| `PluginMetadata::depends_on(id, optional)` | `depends_on(id)` and `optional_dependency(id)`, as in the native API |

```rust
// 0.2.3
fn on_enable(&self, ctx: &Context) -> Result<(), String> {
    std::fs::write("state.txt", "x").map_err(|e| e.to_string())?;
    Ok(())
}

// 0.3.0
fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
    std::fs::write("state.txt", "x")?;
    Ok(())
}
```

## The `#[plugin]` macro

- `#[plugin(depends = ["auth"], soft_depends = ["stats"])]` declares dependencies; 0.2.3 always sent an empty list.
- The plugin id is checked at compile time: lowercase letters, digits, `-` and `_`, starting with a letter or a digit, at most 64 characters. That includes the id derived from the package name; set `id = "..."` if your package name does not qualify.
- The generated glue targets the 0.3 exports and needs no `unsafe` from you: `#![forbid(unsafe_code)]` compiles.

## Errors

0.2.3 had `PlayerError` and `ServiceError`, and many calls had no error at all: a refused `subscribe` returned a dead handle, a refused `register` did nothing, a refused `delay` returned `0`. In 0.3 every host call that can fail returns `Result<_, Error>`, with an `ErrorKind` to branch on.

| 0.2.3 | 0.3.0 |
|-------|-------|
| `ServiceError::OperationFailed("missing capability: ban")` | `Error { kind: PermissionDenied, message: "missing capability: ban" }` |
| `ServiceError::Unavailable("host call timed out")` | `Error { kind: Timeout, .. }` |
| `ServiceError::NotFound(..)` | `ErrorKind::NotFound` |
| `PlayerError::Disconnected`, player not found | `ErrorKind::PlayerGone` |
| `PlayerError::SwitchFailed("host call timed out")` | `ErrorKind::Timeout` |
| `PlayerError::NotActive` | `ErrorKind::InvalidState` |
| a malformed UUID trapped the plugin | impossible: UUIDs are `Uuid` values |
| registrations had no limit | `ErrorKind::LimitExceeded` past the plugin's `[wasm.quotas]` (listeners, commands, tasks, channels, codec filters, limbo handlers) |

The player reads (`Players::get`, `by_name`, `by_uuid`, `list`, `on_server`, `by_ip`, `count`) still answer `None`, an empty list or `0` without `player-read`.

## Events

### Subscribing

`ctx.on` now returns `Result<EventSubscription, Error>`: add `?` or handle the error. A subscription to `ChatMessageEvent` without `chat-intercept` is an error instead of a silent dead handle.

### Results

In 0.2.3 each handler started from an empty result and `allow()` meant "no change", so a later handler could not undo an earlier deny. In 0.3 the handler reads the current result and every helper sets it:

| 0.2.3 | 0.3.0 |
|-------|-------|
| result invisible to the handler | `event.result()` reads it |
| `allow()` cleared the guest's own result: no effect on the event | `allow()` sets the result to allowed, overriding earlier handlers |
| untouched handler sent `none` | untouched handler sends `unchanged` (no effect), as before |
| `ProxyPingEvent::response` field, always sent back | one method per field (`max_players()`, `set_max_players(n)`, `description()`, `set_description(c)`, ...); only the fields you set are sent back. `response()` and `set_response(r)` handle the whole `PingResponse` |

A ping handler written against `response_mut()` becomes, for example:

```rust
ctx.on::<ProxyPingEvent>(EventPriority::NORMAL, |event| {
    event.set_max_players(event.online_players() + 1);
    event.set_description("Welcome");
})?;
```

The description, favicon and player sample are read from the host the first time the handler asks for them, so a handler that only changes the counts no longer copies them into the plugin on every ping. At the WIT level, `proxy-ping-result` carries `max-players`, `online-players`, `protocol` and `version-name`; its `description`, `favicon` and `player-sample` are `none` in the event and set only what changed in an outcome, and the guest reads the current ones with `events.ping-description`, `ping-favicon` and `ping-player-sample`. See [The ping response](./api-reference#the-ping-response).

A handler that only logs is unchanged in behaviour. A handler that called `allow()` to express "I have no objection" now actively allows: remove the call if it should not override other plugins.

### Renamed and reshaped events

| 0.2.3 | 0.3.0 |
|-------|-------|
| `player_id: u64`, `username` on player events | `player: PlayerRef` with `id: PlayerId`, `uuid: Uuid`, `username` |
| `profile` on server-pre-connect, player-choose-initial-server, permissions-setup | removed: read `player.username`, or `Players::get(player.id)` for more |
| `ServerSwitchEvent { previous_server, new_server }`, only for switches | `ServerPostConnectEvent { server, previous_server }` for every post-connect; `switched_from()` answers the previous server of a real switch |
| `ServerPreConnectEvent::original_server` | `server`, plus `previous_server` and `cause: ConnectCause` |
| `ServerConnectedEvent { player_id, server }` | adds `previous_server` |
| `KickedFromServerEvent::reason: String` (JSON, empty when none) | `reason: Option<Component>`, plus `cause: KickCause`, `during_connect`, `previous_server` |
| `KickedFromServerEvent::disconnect_player(reason)` | `disconnect(reason)` |
| `PlayerChooseInitialServerEvent::redirect(server)` | `redirect_to(server)`; `deny(reason)` disconnects the player, and an earlier deny shows as `PlayerChooseInitialServerResult::Denied` |
| no `PlayerInfo::settings`, `known_channels` | `settings: Option<ClientSettings>`, `known_channels: Vec<String>` |
| `ChatMessageEvent { player_id, message }` | adds `signed` and `server`; `deny_silently()` denies without a reason |
| `DisconnectEvent { player_id, username, last_server }` | `player`, `last_server: Option<ServerId>`, `cause: DisconnectCause` |
| `PreLoginEvent::remote_addr: String`, `protocol_version` | `remote_addr: SocketAddr`, `protocol` |
| `ProxyPingEvent::remote_addr: String`, `response.protocol_version` | `remote_addr: SocketAddr`, `server`, `virtual_host`, `protocol`, `legacy`; the response has `protocol` and `player_sample` |
| `ConfigReloadEvent` (no fields) | `provider`, `added`, `removed`, `updated` |
| none | `BackendHealthEvent { address, servers, state }` |
| `raw-packet` event kind (never delivered) | `RawPacketEvent`, delivered through `ctx.on_packets(filters, priority, handler)` with the `raw-packet` capability |
| `permissions-setup` `custom(handler-id)` outcome, `permission-level-of`, `check-permission` exports (never wired) | removed; `PermissionsSetupEvent::provide(PermissionSnapshot)` installs a custom checker, see [Permissions](./permissions) |

Every reason and message argument takes `impl Into<Component>`, so `deny("Banned")` still compiles, and it now means plain text rather than JSON.

### Events new in 0.3

0.2.3 only exposed the lifecycle, connection, chat and proxy events. 0.3 exposes every native event a plugin can subscribe to, so these have no 0.2.3 equivalent to migrate from:

| Event | Result | Capability beyond `event-bus` |
|-------|--------|-------------------------------|
| `ConnectionHandshakeEvent` | allow, deny, drop silently | |
| `ConnectionRejectedEvent` | none | |
| `GameProfileRequestEvent` | the profile, and a deny (`deny`, `denied`, `allow`) | |
| `LoginEvent` | allowed, denied | |
| `CommandExecuteEvent` | allow, deny, modify, forward to backend | `chat-intercept` |
| `LimboEnterEvent`, `LimboExitEvent` | none | |
| `PlayerClientBrandEvent`, `PlayerSettingsChangedEvent`, `PlayerChannelRegisterEvent`, `PlayerResourcePackStatusEvent` | none | |
| `PluginMessageEvent` | forward, handled, replace | `plugin-messaging` |
| `BanIssuedEvent`, `BanRevokedEvent` | none | |
| `PluginEnabledEvent`, `PluginDisabledEvent` | none | |
| `PreTransferEvent` | allowed, denied, redirect | |
| `NamedEvent` | cancelled, response | |
| `RawPacketEvent` | pass, modify, drop | `raw-packet` |

`NamedEvent` is the custom event native plugins exchange: subscribe with `ctx.on_named(name, ..)`, fire with `ctx.fire_named(..)`. See [Named events](./events#named-events).

## Players

0.2.3 handed out a `Player` resource. 0.3 addresses players by id: lookups return a `PlayerInfo` snapshot and actions go through a `Player` handle that is only the id.

| 0.2.3 | 0.3.0 |
|-------|-------|
| `Players.online_count()`, `online_count_on(server)` | `Players::count()`, `Players::count_on(&server)` |
| `Players.get_by_id(id)` | `Players::get(PlayerId)` |
| `Players.get_by_name(name)`, `get_by_uuid(&str)` | `Players::by_name(name)`, `Players::by_uuid(Uuid)` |
| `Players.on_server(server)`, `all()` | `Players::on_server(&ServerId)`, `Players::list()`, which answer a `PlayerSummary` (`player: PlayerRef`, `current_server`) per player; `summary.info()` or `Players::get(id)` for the full `PlayerInfo` |
| `ctx.player_registry()` | the `Players` functions directly |
| none | `Players::by_ip(IpAddr)`, a `PlayerSummary` per player connected from that address |
| `player.profile()`, `remote_addr()`, `current_server()`, ... | fields of `PlayerInfo`: `profile`, `remote_addr: SocketAddr`, `current_server`, `online_mode`, `connected`, `active`, `connected_at: SystemTime`, plus `virtual_host`, `client_brand`, `ping` |
| `player.permission_level()` | removed: use `has_permission("infrarust.admin")` |
| `player.has_permission(node) -> bool` | `Player::has_permission(node) -> Result<bool, Error>` |
| `player.send_message(&component.into_json())` | `player.send_message(component)` |
| `player.disconnect(&json)` returned nothing | `Player::disconnect(reason) -> Result<(), Error>` |
| `player.send_packet(&RawPacket)` | `Player::send_packet(packet_id, data)` |
| none | `Player::connect` (waits for the switch outcome), `set_player_list_header_footer`, `clear_title`, `show_boss_bar`, `send_resource_pack`, `remove_resource_pack`, `transfer`, `store_cookie`, `request_cookie`, `refresh_permissions`, all `player-write` |

```rust
// 0.2.3
if let Some(id) = invocation.player
    && let Some(player) = Players.get_by_id(id)
{
    let _ = player.send_message(&Component::text(&reply).into_json());
}

// 0.3.0
let _ = invocation.reply(Component::text(reply));
```

## Text components

| 0.2.3 | 0.3.0 |
|-------|-------|
| `Component` built a JSON string: `text`, `color`, `bold`, `italic`, `append` | full builder at parity with native: translatable, keybind, hex colors, every decoration, font, insertion, shadow color, click and hover events |
| `into_json()` / `String::from(component)` | `to_json()` asks the host and returns `Result<String, Error>` |
| any `String` passed as a message was parsed as JSON, or shown as text when it did not parse | a `String` is always plain text; parse JSON with `Component::from_json` and legacy `&` codes with `Component::from_legacy` |
| no limits | at most 4096 nodes, 64 levels and 256 KiB of text; a larger component is refused with `InvalidArgument` |

## Commands

| 0.2.3 | 0.3.0 |
|-------|-------|
| `ctx.command(name, handler)` | `ctx.command(name).handler(handler)` |
| `.register()` returned nothing | `.register()` returns `Result<CommandRegistration, Error>`, with the registered name and the rejected aliases |
| a refused registration was only logged, and the guest kept its handler | `Err(Conflict)` and the guest keeps nothing, so `unregister_command` answers `false` |
| `CommandInvocation { args, player: Option<u64> }` | `CommandInvocation { label, args, raw, sender: CommandSender }`, `player()` and `reply(message)` |
| `.completer(\|parts: &[String], cursor: u32\| -> Vec<String>)` | `.completer(\|completion: &Completion\| -> Vec<impl Into<Suggestion>>)`, with `partial()`, the sender and tooltips |
| no permission, usage or hidden flag | `.permission(node)`, `.usage(text)`, `.hidden(true)` |
| `ctx.unregister_command(name) -> bool` | `-> Result<bool, Error>` |
| no way to read the command table | `Commands::get`, `get_by_name`, `get_by_alias` and `contains` look a command up by label; `list` returns every command and `list_owned` only your plugin's, as `CommandInfo` |

## Scheduler

| 0.2.3 | 0.3.0 |
|-------|-------|
| `ctx.delay(..) -> TaskHandle` (a `u64`, `0` when refused) | `-> Result<TaskHandle, Error>` |
| `ctx.interval(..) -> TaskHandle` | `-> Result<TaskHandle, Error>`; `interval_with_delay` sets the first run |
| `ctx.cancel(handle)` | `ctx.cancel(handle)` or `handle.cancel()` |

## Services

| 0.2.3 | 0.3.0 |
|-------|-------|
| `Servers.state(&str) -> Option<ServerState>` | `Servers::state(&ServerId) -> Result<Option<ServerState>, Error>` |
| `Servers.all() -> Vec<(String, ServerState)>` | `Servers::list() -> Result<Vec<ServerStatus>, Error>` |
| `Bans.ban(&target, reason, duration_ms)` | `Bans::ban(BanRequest::new(target).reason(..).duration(..))`, which answers the `BanEntry`, with `kick` and `silent` flags |
| `Bans.unban(&target) -> bool` | `Bans::unban(&target) -> Result<Option<BanEntry>, Error>` |
| `Bans.get(&target)`, `is_banned(&target)` | `Bans::get`, `Bans::is_banned` |
| `Bans.all()` | `Bans::list(cursor, limit)`, paged |
| `BanTarget::Ip(String)` with an optional CIDR | `BanTarget::Ip(IpAddr)` and `BanTarget::IpRange(String)` |
| `BanTarget::Uuid(String)` | `BanTarget::Uuid(Uuid)` |
| `BanEntry` without an id, times in epoch millis | `BanEntry { id, .., created_at: SystemTime, expires_at: Option<SystemTime> }` |
| `Config.get(key) -> Option<String>` | `Config::get(key) -> Result<Option<String>, Error>` |
| `Config.server(..)`, `servers()` | `Config::server(&ServerId)`, `Config::servers()`, both `Result` |
| `ctx.server_manager()`, `ban_service()`, `config_service()` | the `Servers`, `Bans` and `Config` functions directly |
| none | `Config::server_by_domain`, `server_document`, `server_sources`, `proxy_document`, `effective_proxy_document` (`config-read`) and `write_proxy_document` (`config-write`) |
| none | `LoadBalancer` (`config-read` to read, `server-manage` to drain and reset) |
| none | `Messaging` for plugin channels (`plugin-messaging`) |
| none | `Proxy` (version, limits, granted capabilities) and `Plugins` (loaded plugins), always available |
| none | `ctx.provide_bans(impl BanProvider)` makes the plugin the ban provider (`ban-provider`, see [Bans](./bans)) |
| none | `ctx.provide_permissions(impl PermissionProvider)` makes the plugin the permission provider, `Permissions::set_snapshot` and `release` change a player's permissions live (`permission-provider`, see [Permissions](./permissions)) |

The contract gained the `permissions` and `providers` interfaces, the provider records in `ban-service` (`login-attempt`, `ban-record`, `ban-verdict` and their companions), the `custom(permission-snapshot)` case of `permissions-setup-result`, and six guest exports: `ban-provider-check`, `ban-provider-ban`, `ban-provider-unban`, `ban-provider-get`, `ban-provider-list` and `permission-snapshot-for`. The `#[plugin]` macro generates the exports; a plugin that provides nothing answers them with an error or an empty snapshot. A hand-written `Guest` implementation must add them.

## Limbo

| 0.2.3 | 0.3.0 |
|-------|-------|
| `session.player_id() -> u64` | `-> PlayerId` |
| `session.send_message(Component)`, `send_title(TitleData)` (JSON title) | `send_message(impl Into<Component>)`, `send_title(&TitleData)` with component lines |
| `session.complete(outcome)` returned nothing | `-> Result<(), Error>` |
| `HandlerOutcome::Redirect(String)`, `TimeoutOutcome::Redirect(String)` | `Redirect(ServerId)`: `"hub".into()` |
| `EntryContext::KickedFromServer { server: String, reason: String }` | `{ server: ServerId, reason: Component }` |
| `on_disconnect(&self, player_id: u64)`, `on_session_end(&self, player_id: u64, ..)` | `PlayerId` |

## Codec filters

The filter trait is unchanged. `CodecSessionInit` now carries `remote_addr: SocketAddr` and `real_ip: Option<IpAddr>` instead of strings.

`CodecRegistrar::add_required` registers a filter the proxy must not do without: when it traps, cannot be created, or is quarantined for the client's address, the connection is closed or refused instead of passing unfiltered. At the WIT level `codec-filter-metadata` gains a `required: bool` field in the 0.3.0 contract; a guest that builds the record by hand sets it (`false` keeps the fail-open behaviour). See [Required filters](./codec-filters#required-filters).

## Types

The SDK now exports `PlayerId`, `ServerId`, `PlayerRef`, `GameProfile`, `ServerAddress`, `ServerState`, `ProxyMode`, `ChannelId`, `ClientSettings`, `PacketDirection`, `Capability` and re-exports `Uuid`. `ServerId` converts from `&str` and `String`, so `redirect_to("lobby")` still compiles.

The colours, `Capability` and the data-only enums (`ServerState`, `ProxyMode`, `PacketDirection`, `ChatMode`, `MainHand`, `ParticleStatus`, `HandshakeIntent`, `ConnectCause`, `TransferOrigin`, `LoginStage`, `SessionEndReason`, `BackendState`, `ResourcePackStatus`, `FilterPriority`, `MessagePhase`, `UnknownDomainBehavior`) are the same types as in the native `infrarust-api`: both crates re-export them from `infrarust-plugin-common`. Their SDK paths are unchanged. `Capability::as_str` is gone: use `Capability::to_kebab()` (or `to_string()`, the `Display` impl prints the same kebab-case name), and `"chat-intercept".parse::<Capability>()` works. `FilterPriority` now derives `PartialEq`, `Eq`, `Ord` and `Hash`, and the shared enums are `#[non_exhaustive]`, so a `match` on them needs a wildcard arm.

## Behaviour changes without a build break

These do not show up as compile errors, so check them by hand:

- **Import gate.** The host checks every capability a host function needs when the component is loaded, not only when the function is called. A plugin that imports `subscribe-packets` needs both `raw-packet` and `event-bus`; with one of them missing it is refused at load, and the report names each missing capability. Under 0.2.3 such a plugin loaded and then had every subscription refused.
- **Ban ranges in a guest `BanProvider`.** The SDK's `ip_in_range` now matches v4-mapped ranges (`::ffff:10.0.0.0/104` is `10.0.0.0/8`) and compares usernames with the same Unicode lowercase the native api uses, so a guest provider and the built-in store agree on what a ban matches.
- **Limbo handlers across a recovery.** When a plugin is restarted after a trap, the limbo handler names its new generation does not register again are released instead of staying taken and answering "unavailable".
- **Registration quotas.** A plugin holds at most 1024 event listeners, 256 commands, 1024 live scheduled tasks, 128 plugin channels, 32 codec filters and 64 limbo handlers by default. Past a quota the registration returns `ErrorKind::LimitExceeded`, a new `error-kind` case (`limit-exceeded`) in the 0.3.0 contract; a `match` on `ErrorKind` needs a wildcard arm, since the enum is `#[non_exhaustive]`. The operator raises a quota under `[wasm.quotas]` or `[plugins.<id>.wasm.quotas]`.
- **Config reads are scoped to the plugin.** `Config::proxy_document` and `effective_proxy_document` no longer carry the other plugins' `[plugins.<id>]` blocks, `Config::get` on a key under another plugin returns `ErrorKind::PermissionDenied`, and `Config::get("plugins")` holds the plugin's own entry only. `write_proxy_document` puts the hidden blocks back and returns `PermissionDenied` for a document that carries another plugin's block. A plugin can no longer read another plugin's configuration through `config-read`.
- **`complete` does not start a new hold.** `session.complete` and `handle.complete` with `HandlerOutcome::Hold` or `HoldWithTimeout` return `ErrorKind::InvalidArgument` and leave the current hold and its deadline in place. Under 0.2.3 the host released the player as for `Accept`. See [Limbo](./limbo#completing-a-hold).
- **Intervals wait for their run.** An interval's next run starts one period after the previous run returns, like a native repeating task, and at most one run of an interval waits in the plugin's queue.
- **Shared enums.** `ServerState`, `ProxyMode`, `Capability` and the other enums listed under [Types](#types) are the same types as in the native api and are `#[non_exhaustive]`.
- **Codec budget and quarantine.** The default `codec_cpu_budget` is now 5 ms instead of 800 ms, and the sandbox clock (`epoch_tick`) ticks every 1 ms instead of 50 ms. Budgets count the ticks during which the guest runs, from the start of each call. A filter that needs more than 5 ms for some packets traps under 0.3 where it passed under 0.2.3; raise `codec_cpu_budget` for its plugin under `[plugins.<id>.wasm]`. A filter that traps 5 times within 10 s for one client address is now quarantined for that address (`[wasm.codec_quarantine]`). A configuration that sets `epoch_tick` explicitly above 5 ms keeps working, with a warning: each codec call then gets one tick.
- **Plugins in subfolders.** The proxy loads only the `.wasm` files directly in `plugins_dir`; subdirectories are no longer scanned, because each plugin's data directory is one of them. If you organised plugins in subfolders, move the files up to `plugins_dir`, or leave a symlink there pointing to each one.
- **The compile cache moved.** Compiled plugins are cached in `[wasm] cache_dir`, `./cache/wasm` in the working directory by default, instead of `plugins_dir/.cache`. The old directory is neither read nor migrated, so each plugin is compiled once more at the first start; the proxy then logs that `plugins_dir/.cache` can be deleted. A `cache_dir` inside `plugins_dir` is refused at startup. When the proxy cannot write `cache_dir` (a read-only filesystem, a working directory the proxy user does not own), it compiles the plugins in memory at each start and says so once. The Docker image creates the default `/app/cache/wasm` for its user, see [Docker](../../guide/docker#directory-structure).
- **Log macros skip disabled levels.** `trace!`, `debug!`, `info!`, `warn!` and `error!` format their message only when the proxy logs that level: the SDK asks the host once per instance through the new `log.max-level` function. Arguments with side effects are therefore not evaluated at a level the proxy does not log, as with the native `tracing` macros. A guest that calls the `log` functions directly still sends every line.
- **Plugin health in the registry.** `Plugins::list` and `get` fill `PluginInfo::health` for WASM plugins (healthy, recovering, quarantined or stopped), the `plugin-info.health` field of the contract; `state` keeps the lifecycle state.
- **Only binary components.** A `.wasm` file must start with the component header. WebAssembly text and core modules are refused with their own reason, and so is a file larger than 256 MiB.

## Checklist

1. Bump `infrarust-plugin-sdk` and rebuild for `wasm32-wasip2`.
2. Change `Result<(), String>` to `Result<(), PluginError>` and drop the `map_err(|e| e.to_string())` calls.
3. Add `?` to `ctx.on`, `register()`, `ctx.delay` and `ctx.interval`.
4. Rewrite `ctx.command(name, handler)` as `ctx.command(name).handler(handler)`.
5. Replace `player_id` fields with `player.id`, `Players.get_by_id` with `Players::get`, and resource calls with `Player` handle calls.
6. Remove `into_json()` calls; pass components or strings directly.
7. Replace `Capability::as_str()` with `to_kebab()`, and give every `match` on a shared enum a wildcard arm.
8. Review every `allow()`: it now overrides earlier handlers.
9. Replace `ServerSwitchEvent` with `ServerPostConnectEvent` and `switched_from()`.
10. Add `#![forbid(unsafe_code)]` to your crate root.
11. Grant every capability the plugin's imports need: `plugin-messaging` for plugin channels, `chat-intercept` for command listeners, `raw-packet` and `event-bus` for packet subscriptions.
12. Grant `ban-provider` or `permission-provider`, and select the plugin in `[ban] provider` or `[permissions] provider`, for a plugin that provides bans or permissions.

## See also

- [API Reference](./api-reference): the full 0.3.0 contract.
- [Events](./events): results and the event reference.
- [Services](./services): players, errors and text components.
- [Commands](./commands): the command builder.
