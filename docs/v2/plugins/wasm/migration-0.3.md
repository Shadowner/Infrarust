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

The player reads (`Players::get`, `by_name`, `by_uuid`, `list`, `on_server`, `count`) still answer `None`, an empty list or `0` without `player-read`.

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
| `ProxyPingEvent::response` field, always sent back | `response()` reads, `response_mut()` or `set_response()` changes and sends it back |

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
| `PlayerChooseInitialServerEvent::redirect(server)` | `redirect_to(server)` |
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
| `GameProfileRequestEvent` | the profile | |
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
| `Players.on_server(server)`, `all()` | `Players::on_server(&ServerId)`, `Players::list()` |
| `ctx.player_registry()` | the `Players` functions directly |
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
| none | `Config::server_document`, `server_sources`, `proxy_document`, `effective_proxy_document` (`config-read`) and `write_proxy_document` (`config-write`) |
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

## Types

The SDK now exports `PlayerId`, `ServerId`, `PlayerRef`, `GameProfile`, `ServerAddress`, `ServerState`, `ProxyMode`, `ChannelId`, `ClientSettings`, `PacketDirection`, `Capability` and re-exports `Uuid`. `ServerId` converts from `&str` and `String`, so `redirect_to("lobby")` still compiles.

## Checklist

1. Bump `infrarust-plugin-sdk` and rebuild for `wasm32-wasip2`.
2. Change `Result<(), String>` to `Result<(), PluginError>` and drop the `map_err(|e| e.to_string())` calls.
3. Add `?` to `ctx.on`, `register()`, `ctx.delay` and `ctx.interval`.
4. Rewrite `ctx.command(name, handler)` as `ctx.command(name).handler(handler)`.
5. Replace `player_id` fields with `player.id`, `Players.get_by_id` with `Players::get`, and resource calls with `Player` handle calls.
6. Remove `into_json()` calls; pass components or strings directly.
7. Review every `allow()`: it now overrides earlier handlers.
8. Replace `ServerSwitchEvent` with `ServerPostConnectEvent` and `switched_from()`.
9. Add `#![forbid(unsafe_code)]` to your crate root.
10. Grant `plugin-messaging` to a plugin that uses plugin channels, and `chat-intercept` to one that listens to commands.
11. Grant `ban-provider` or `permission-provider`, and select the plugin in `[ban] provider` or `[permissions] provider`, for a plugin that provides bans or permissions.

## See also

- [API Reference](./api-reference): the full 0.3.0 contract.
- [Events](./events): results and the event reference.
- [Services](./services): players, errors and text components.
- [Commands](./commands): the command builder.
