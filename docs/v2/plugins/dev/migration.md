---
title: Migrating from 2.0.0-beta.3
description: Port a native plugin written against the infrarust-api of 2.0.0-beta.3 to the reworked plugin API, area by area, with what to change and why.
outline: [2, 3]
---

# Migrating from 2.0.0-beta.3

The plugin system was reworked after 2.0.0-beta.3. The native API in `infrarust-api` breaks in most areas: events carry the player handle instead of an id, commands have owners and a source, permissions are nodes answered by a provider, the ban service is a replaceable provider, and text components model the whole format. This page lists every change a native plugin has to make, with the beta.3 code, the current code and what to do.

WASM plugins moved from the `infrarust:plugin@0.2.3` contract to `0.3.0` at the same time. They have their own guide: [Migrating to 0.3](../wasm/migration-0.3).

Start by bumping the dependency and building. The compiler finds most of what follows. The items marked **Compiles unchanged** do not break the build but change what your plugin sees at run time, so read them even when the build is green.

In the snippets, `// 2.0.0-beta.3` marks the old code and `// current` the code that builds against the reworked API.

## Events

### The player handle replaces the player id

Every event about a logged-in player now carries `player: Arc<dyn Player>`, the same handle the player registry returns. The `player_id` field became a `player_id()` method, and fields the handle already exposes (profile, username) moved behind it.

| Event | 2.0.0-beta.3 fields | Current fields |
|-------|---------------------|----------------|
| `PostLoginEvent` | `profile`, `player_id`, `protocol_version` | `player`, `profile`, `protocol_version`; `player_id()` |
| `DisconnectEvent` | `player_id`, `username`, `last_server` | `player`, `last_server`, `cause`; `player_id()`, `username()` |
| `PermissionsSetupEvent` | `player_id`, `profile`, `online_mode` | `player`, `online_mode`; `player_id()`, `profile()` |
| `PlayerChooseInitialServerEvent` | `player_id`, `profile`, `initial_server` | `player`, `initial_server`; `player_id()`, `profile()` |
| `ServerPreConnectEvent` | `player_id`, `profile`, `original_server` | `player`, `server`, `previous_server`, `cause`; `player_id()`, `profile()` |
| `ServerConnectedEvent` | `player_id`, `server` | `player`, `server`, `previous_server`; `player_id()` |
| `KickedFromServerEvent` | `player_id`, `server`, `reason` | `player`, `server`, `reason`, `cause`, `during_connect`, `previous_server`; `player_id()`, `profile()` |
| `ChatMessageEvent` | `player_id`, `message` | `player`, `message`, `signed`, `server`; `player_id()`, `profile()` |

```rust
// 2.0.0-beta.3
let registry = ctx.player_registry_handle();
bus.subscribe::<PostLoginEvent, _>(EventPriority::NORMAL, move |event| {
    if let Some(player) = registry.get_player_by_id(event.player_id) {
        let _ = player.send_message(Component::text("Welcome!"));
    }
});

// current
bus.subscribe::<PostLoginEvent, _>(EventPriority::NORMAL, |event| {
    tracing::info!("{} joined ({:?})", event.profile.username, event.player_id());
    let _ = event.player.send_message(Component::text("Welcome!"));
});
```

What to do: replace `event.player_id` with `event.player_id()`, `event.username` with `event.username()`, and `event.profile` with `event.profile()` where the table says so. Drop the registry lookups that only turned the id back into a player.

Outside `infrarust-api` you can no longer build these events with a struct literal. Most of them are `#[non_exhaustive]` now, as are `ServerPostConnectEvent`, `ProxyPingEvent`, `ConfigReloadEvent` and the new events; `PermissionsSetupEvent`, `PlayerChooseInitialServerEvent` and `ServerPreConnectEvent` are not, but a private `result` field has the same effect. Tests build them all with their `new` constructor, with a `MockPlayer` from the `test-util` feature for the player. See [Testing](./testing#mockplayer).

### `ServerSwitchEvent` is removed

A server connection is now announced by three awaited events, in this order: `ServerPreConnectEvent` before the proxy opens any connection to the target, `ServerConnectedEvent` once the backend accepted the login, and `ServerPostConnectEvent` (new) once the backend's `JoinGame` reached the client and `current_server()` moved. A switch is a `ServerPostConnectEvent` whose `previous_server` differs from `server`, which `switched_from()` returns.

```rust
// 2.0.0-beta.3
bus.subscribe::<ServerSwitchEvent, _>(EventPriority::NORMAL, |event| {
    tracing::info!("{:?} moved from {} to {}", event.player_id, event.previous_server, event.new_server);
});

// current
bus.subscribe::<ServerPostConnectEvent, _>(EventPriority::NORMAL, |event| {
    if let Some(from) = event.switched_from() {
        tracing::info!(
            "{} moved from {from} to {}",
            event.player.profile().username,
            event.server
        );
    }
});
```

What to do: subscribe to `ServerPostConnectEvent`. Use `switched_from()` to keep only switches; the first connection has `previous_server: None`.

### The connection events

`ServerPreConnectEvent`:

- `original_server` is now `server`. The event adds `previous_server` and `cause: ConnectCause` (`Initial`, `Switch`, `LimboExit`, `KickRedirect`, `PluginMessage`).
- `ServerPreConnectResult::VirtualBackend` is removed: nothing honoured it.
- It fires exactly once per connection attempt. In beta.3 a limbo gate on the first server fired it twice, plus a switch from the server to itself. A switch to the server the player is already on now fires nothing.
- For passthrough, `zero_copy` and `server_only` players, `ConnectTo` is now honoured; beta.3 honoured only `Denied` for them.

`ServerConnectedEvent` (**Compiles unchanged** apart from the fields): beta.3 fired it detached, right after the TCP connection opened, even when the backend then refused the login. It is now awaited and fires once the backend accepted the login.

```rust
// 2.0.0-beta.3
bus.subscribe::<ServerPreConnectEvent, _>(EventPriority::NORMAL, |event| {
    if event.original_server.as_str() == "maintenance" {
        event.redirect_to(ServerId::new("lobby"));
    }
});

// current
bus.subscribe::<ServerPreConnectEvent, _>(EventPriority::NORMAL, |event| {
    if event.cause == ConnectCause::Initial && event.server.as_str() == "maintenance" {
        event.redirect_to(ServerId::new("lobby"));
    }
});
```

What to do: rename `original_server` to `server` and drop any `VirtualBackend` arm. If a `ServerConnectedEvent` listener relied on running before the backend answered, move it to `ServerPreConnectEvent`. See [Server connections](./events#server-connections).

### `KickedFromServerEvent`

| | 2.0.0-beta.3 | Current |
|-|--------------|---------|
| When it fires | After the backend's disconnect packet was already forwarded to the client | Before anything of the kick reaches the client, for play, login and configuration disconnects, lost connections, refused logins and unreachable servers (in the forwarding modes, only for an unreachable server) |
| `reason` | `Component` (the raw JSON or NBT wrapped as text) | `Option<Component>`, parsed |
| New fields | | `cause: KickCause` (`Unreachable { error }`, `LoginRefused`, `ConfigDisconnect`, `PlayDisconnect`, `ConnectionLost`), `during_connect`, `previous_server` |
| `DisconnectPlayer` | `{ reason: Component }` | `{ reason: Option<Component> }`; `None` shows the client the server's own disconnect |
| Starting result | Always `DisconnectPlayer { reason: "Kicked from server" }` | Depends on the situation, see below |
| `Notify` | Disconnected the player anyway | Keeps the player on the server they are on and sends the message |
| Shortcuts | `redirect_to` | `redirect_to`, `disconnect`, `send_to_limbo`, `notify` |

The starting result follows the situation: a kick in play forwards the backend's packet byte for byte (`DisconnectPlayer { reason: None }`), a failed switch keeps the player on their server and notifies them, and a failed first connection sends the player to the server's limbo handlers when it has some. `RedirectTo` goes through `ServerPreConnectEvent` with the `KickRedirect` cause and stops after three redirects in a row.

```rust
// 2.0.0-beta.3
bus.subscribe::<KickedFromServerEvent, _>(EventPriority::NORMAL, |event| {
    tracing::info!("{:?} kicked from {}: {}", event.player_id, event.server, event.reason);
    event.redirect_to(ServerId::new("lobby"));
});

// current
bus.subscribe::<KickedFromServerEvent, _>(EventPriority::NORMAL, |event| {
    let reason = event
        .reason
        .as_ref()
        .map(Component::to_plain)
        .unwrap_or_default();
    tracing::info!(
        "{} left {} ({}): {reason}",
        event.profile().username,
        event.server,
        event.cause.as_str()
    );
    if !event.during_connect {
        event.redirect_to(ServerId::new("lobby"));
    }
});
```

What to do: handle `reason` as an `Option`, wrap `DisconnectPlayer` reasons in `Some`, and read `event.result()` instead of assuming the default is a disconnect. Redirects now work for every kick, not only for a backend that dropped the connection silently. See [KickedFromServerEvent](./events#kickedfromserverevent).

### `DisconnectEvent` and `PostLoginEvent`

`DisconnectEvent` adds `cause: DisconnectCause` (`ClientQuit`, `Kicked { reason }`, `BackendClosed { reason }`, `Shutdown`, `Error`) with `as_str()` and `reason()`, and takes the player handle (see the table above).

```rust
// 2.0.0-beta.3
bus.subscribe::<DisconnectEvent, _>(EventPriority::NORMAL, |event| {
    tracing::info!("{} left", event.username);
});

// current
bus.subscribe::<DisconnectEvent, _>(EventPriority::NORMAL, |event| {
    tracing::info!("{} left: {}", event.username(), event.cause.as_str());
});
```

**Compiles unchanged**, and matters for plugins that track online players:

- beta.3 fired `PostLoginEvent` detached, from inside authentication: before the post-auth ban check, before the player was registered, and with no `DisconnectEvent` when the login then failed. It is now awaited, fires after registration, and a `DisconnectEvent` follows it exactly once. A slow `PostLoginEvent` listener now delays the join, each listener up to `[events] handler_timeout`.
- The login runs in a fixed order: `PreLoginEvent`, authentication, `GameProfileRequestEvent` (new), the post-auth ban check, `PermissionsSetupEvent`, `LoginEvent` (new), registration, `PostLoginEvent`, `PlayerChooseInitialServerEvent`, then the connection events.
- Passthrough, `zero_copy` and `server_only` players go through the same flow; in beta.3 they fired only `PostLoginEvent`, the connection events and `DisconnectEvent`. Pre-1.7 clients, which beta.3 never registered, go through it too.
- `DisconnectEvent` is bounded as a whole by `[events] disconnect_deadline` (15 s): listeners still running then are cancelled and the player is removed anyway.
- At shutdown every player's `DisconnectEvent` fires with the cause `Shutdown` while plugins are still enabled, then `ProxyShutdownEvent`, then `on_disable`. In beta.3 plugins were disabled before connections drained, so these `DisconnectEvent`s reached no plugin.
- A second login with a UUID that is already online disconnects the first session and waits for its `DisconnectEvent` before the new `PostLoginEvent`.

What to do: rename the fields; remove workarounds for ghost players (a `PostLoginEvent` with no `DisconnectEvent`); keep `PostLoginEvent` listeners fast or move slow work to the scheduler. See [Guarantees](./events#guarantees) and [Proxy shutdown](./events#proxy-shutdown).

### `ChatMessageEvent`

| | 2.0.0-beta.3 | Current |
|-|--------------|---------|
| `Deny` | `{ reason: Component }`; dropped the message without showing the reason | `{ reason: Option<Component> }`; the reason is shown, `None` drops silently |
| `Modify` | `{ new_message }`; ignored, the original was forwarded | `{ message }`; honoured, signed messages included |
| Shortcuts | `deny`, `modify(String)` | `allow`, `deny`, `deny_silently`, `modify(impl Into<String>)` |
| Limbo | No event | Fires with `server: None`, before the limbo handler |

```rust
// 2.0.0-beta.3
bus.subscribe::<ChatMessageEvent, _>(EventPriority::NORMAL, |event| {
    if let ChatMessageResult::Modify { new_message } = event.result() {
        tracing::debug!("{new_message}");
    }
});

// current
bus.subscribe::<ChatMessageEvent, _>(EventPriority::NORMAL, |event| {
    if event.message.contains("badword") {
        event.deny(Component::error("Watch your language"));
    } else if let Some(shout) = event.message.strip_prefix('!') {
        let loud = shout.to_uppercase();
        event.modify(loud);
    }
});
```

What to do: rename `new_message` to `message`, wrap `Deny` reasons in `Some`, and check that your modifications and denials are what you want now that the proxy applies them. Commands typed in chat fire the new `CommandExecuteEvent`, not `ChatMessageEvent`; see [Chat and command events](./events#chat-and-command-events).

### Other reshaped events

- **`ProxyPingEvent`** is `#[non_exhaustive]` and gains `server`, `virtual_host`, `protocol_version` and `legacy`; build it with `ProxyPingEvent::new`. `PingResponse` gains `player_sample: Vec<(String, Uuid)>`. It now fires for legacy pings and, with the default MOTD, for unknown domains.
- **`ConfigReloadEvent`** was a unit struct fired once per changed file. It now carries `provider`, `added`, `removed` and `updated` server ids, fires once per batch of changes from a provider, and only when something changed. A listener that ignores the event (`|_: &mut ConfigReloadEvent|`) still compiles.
- **`RawPacketEvent`** no longer implements `Event`: in beta.3, subscribing to it through `subscribe` registered a listener that never ran. Use `subscribe_packet_typed` or `subscribe_packet_async_typed` with a `PacketFilter`. `result()` and `set_result()` are now inherent methods, so `ResultedEvent` no longer needs to be in scope for them.
- **`PermissionsSetupEvent`**: `PermissionsSetupResult::UseDefault` now keeps the checker the active permission provider built, instead of the config-based checker. See [Permissions](#permissions).

`PreLoginEvent` and `OnlineAuthFailed` are unchanged.

### Server wake and player ids

**Compiles unchanged.** Two behaviours moved:

- A managed server that is asleep is started only once `ServerPreConnectEvent` picked it, after `PreLoginEvent`, authentication and `PlayerChooseInitialServerEvent`. A player refused at login or redirected elsewhere no longer wakes it. A server that cannot start, is stopping or whose provider fails now ends in `KickedFromServerEvent` with the `Unreachable` cause, so a listener can redirect the player or send them to limbo, instead of refusing the login before any event.
- `PlayerId` values come from a per-connection counter, so they have gaps (status pings use ids too). They were never meant to be read as a count; keep treating them as opaque.

### New events

| Event | Resulted | What it reports |
|-------|----------|-----------------|
| `ConnectionHandshakeEvent` | yes | Every connection right after routing, before a player exists; allow, deny or drop silently |
| `ConnectionRejectedEvent` | no | A connection the proxy refused before a player existed, once, with a `RejectReason` |
| `GameProfileRequestEvent` | no | The authenticated profile; listeners may rewrite the UUID, name and properties |
| `LoginEvent` | yes | After the ban check and `PermissionsSetupEvent`; can deny with a reason |
| `CommandExecuteEvent` | yes | Every command in play, backend and signed ones included; allow, deny, rewrite or force forwarding. Not in limbo |
| `ServerPostConnectEvent` | no | The server's `JoinGame` reached the client |
| `LimboEnterEvent`, `LimboExitEvent` | no | A limbo stay, with its handlers, why it ended and where the player goes next |
| `PluginMessageEvent` | yes | A plugin message on a channel a plugin registered |
| `PlayerClientBrandEvent`, `PlayerSettingsChangedEvent`, `PlayerChannelRegisterEvent` | no | Client state changes |
| `PreTransferEvent` | yes | A transfer to another host, from a plugin or a backend |
| `PlayerResourcePackStatusEvent` | no | A client's answer to a resource pack |
| `BanIssuedEvent`, `BanRevokedEvent` | no | A ban issued or revoked through the ban service |
| `PluginEnabledEvent`, `PluginDisabledEvent` | no | The plugin lifecycle |
| `ServiceProvidedEvent`, `ServiceRemovedEvent` | no | Services shared through the service registry |
| `NamedEvent` | no | An event fired by name, with a content type and bytes, for plugins that do not share Rust types |

Each one is described in the [Events Reference](./events).

## Event bus

| | 2.0.0-beta.3 | Current |
|-|--------------|---------|
| `unsubscribe(handle)` | `()`; removed any plugin's listener | `bool`; removes only this plugin's listeners, takes effect even for a dispatch in progress, and logs a warning for a foreign handle |
| `ListenerHandle::new(id)` | Public | Replaced by the hidden `from_raw`; handles cannot be forged |
| A panicking listener | Took down the task that fired the event (a login, a ping) | Logged with the plugin id; the event goes on to the next listener |
| A hung async listener | Blocked the event forever | Cancelled after `[events] handler_timeout` (10 s) |
| Firing your own events | Not possible | `EventBusExt::fire(event)` returns `Result<E, FireError>` |
| Queued proxy events | `ServerStateChangeEvent`, `BackendHealthEvent`, `ConfigReloadEvent` each spawned on their own task, in any order | Delivered one at a time, in the order they were posted |

```rust
// current
pub struct PartyCreated {
    pub leader: PlayerId,
}

impl Event for PartyCreated {}

let bus = ctx.event_bus_handle();
ctx.scheduler().spawn(Box::pin(async move {
    match bus.fire(PartyCreated { leader }).await {
        Ok(event) => tracing::debug!("party of {:?} announced", event.leader),
        Err(e) => tracing::warn!("{e}"),
    }
}));
```

Built-in proxy events are refused with `FireError::Reserved`, so a plugin cannot forge a `PreLoginEvent`.

What to do: stop relying on unsubscribing another plugin's handle; make sure an async listener finishes well within `handler_timeout`; keep blocking work out of synchronous listeners, which cannot be cancelled. The [threading model](./threading) explains where each event runs.

## Commands

| | 2.0.0-beta.3 | Current |
|-|--------------|---------|
| Register | `register(name, aliases, description, handler)`, returning `()` | `register(CommandSpec, handler)`, returning `Result<CommandRegistration, CommandError>` |
| Register with an owner | `register_with_plugin_id(..)` | Removed: every command belongs to the plugin that registered it |
| Unregister | `unregister(name)`, returning `()` | `unregister(name)`, returning `Result<(), CommandError>`; only your own commands |
| List | none | `list()` returns `Vec<CommandInfo>` |
| Execute | `execute(&self, ctx, player_registry)` | `execute(&self, ctx: CommandContext)` |
| `CommandContext` | `player_id: Option<PlayerId>`, `args`, `raw` | `source: CommandSource`, `label`, `args`, `raw_args`, `raw`; `#[non_exhaustive]` |
| Tab completion | `tab_complete(args, cursor) -> Vec<String>`, `tab_complete_for(args, cursor, player_id)` | `suggest(&self, ctx: SuggestContext) -> Vec<Suggestion>`, with tooltips |
| Who runs it | Players only | Players and the console (`CommandSource::Player` or `CommandSource::Console`) |
| Permission | none | `CommandSpec::permission(node)`: a player without it gets a message and the command is neither run nor forwarded |
| Where a player's command runs | Awaited in the player's session | On the player's command queue: one at a time, in order, while the session goes on; cancelled when the player leaves. `suggest` takes the same queue |

```rust
// 2.0.0-beta.3
struct Greet;

impl CommandHandler for Greet {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        players: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(id) = ctx.player_id else { return };
            if let Some(player) = players.get_player_by_id(id) {
                let _ = player.send_message(Component::text("Hello!"));
            }
        })
    }

    fn tab_complete<'a>(&'a self, _args: Vec<String>, _cursor: u32) -> BoxFuture<'a, Vec<String>> {
        Box::pin(async { vec!["world".to_string()] })
    }
}

ctx.command_manager().register("greet", &["hi"], "Say hello", Box::new(Greet));

// current
struct Greet;

impl CommandHandler for Greet {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let name = ctx.source.name().to_string();
            ctx.source
                .send_message(Component::text(format!("Hello, {name}!")));
        })
    }

    fn suggest<'a>(&'a self, ctx: SuggestContext) -> BoxFuture<'a, Vec<Suggestion>> {
        Box::pin(async move {
            ["world", "everyone"]
                .into_iter()
                .filter(|word| word.starts_with(ctx.partial()))
                .map(Suggestion::from)
                .collect()
        })
    }
}

let spec = CommandSpec::new("greet")
    .alias("hi")
    .description("Say hello")
    .permission("greet.command.greet");
if let Err(e) = ctx.command_manager().register(spec, Box::new(Greet)) {
    tracing::warn!("/greet was not registered: {e}");
}
```

Names follow new rules: built-in names are reserved (`CommandError::Reserved`), a bare name goes to the first plugin (`CommandError::OwnedBy`), every plugin command is also reachable as `<plugin_id>:<name>`, and a clashing alias is skipped and reported in `rejected_aliases`. Registering or removing a command updates every connected client's command tree at once.

What to do: build a `CommandSpec`, handle the `Result`, reply through `ctx.source` (`send_message`, `name`, `player`, `has_permission`), and replace `tab_complete` with `suggest`. A handler that needs the registry keeps `ctx.player_registry_handle()` itself. Check `ctx.source.is_console()` if the command only makes sense for a player. See [Commands](./commands).

## Permissions

| | 2.0.0-beta.3 | Current |
|-|--------------|---------|
| Model | Two levels, `PermissionLevel::Player` and `Admin`, from `[permissions] admins` | Permission nodes answered `True`, `False` or `Undefined` (`Tristate`) |
| `PermissionChecker` | `permission_level()` and `has_permission()` | `value(node) -> Tristate`; `has_permission` has a default |
| `Player::permission_level()` | Present | Removed |
| Answering for every player | Only by overriding each player in `PermissionsSetupEvent` | A `PermissionProvider`, registered with `ctx.register_permission_provider` and selected by `[permissions] provider` |
| Plugin nodes | none | `ctx.register_permission_node(PermissionNode::new(name, default))`, default `True`, `False` or `Admin` |
| A node the checker has no answer for | Granted to admins, denied to others | The node's registered default; denied when the node is not registered |
| Changes while online | none | `player.refresh_permissions()` rebuilds the checker and resends the command tree |
| Offline-mode admins | Admins by UUID | Admins only with `[permissions] trust_offline_admins = true` |

```rust
// 2.0.0-beta.3
impl PermissionChecker for StaffChecker {
    fn permission_level(&self) -> PermissionLevel {
        if self.staff { PermissionLevel::Admin } else { PermissionLevel::Player }
    }

    fn has_permission(&self, permission: &str) -> bool {
        self.staff || permission == "lobby.use"
    }
}

// current
impl PermissionChecker for StaffChecker {
    fn value(&self, node: &str) -> Tristate {
        if self.staff || node == "lobby.use" {
            Tristate::True
        } else {
            Tristate::Undefined
        }
    }
}
```

Return `Undefined` rather than `False` for nodes your checker has no opinion on, so the node's registered default applies. Register the nodes your plugin checks:

```rust
// current
ctx.register_permission_node(
    PermissionNode::new("greet.command.greet", PermissionDefault::True)
        .description("Use /greet"),
)
.map_err(|e| PluginError::InitFailed(e.to_string()))?;
```

`PermissionsSetupEvent` still overrides one player's checker with `PermissionsSetupResult::Custom`; it now runs after the provider built the default checker. `Capability` gains `ChatIntercept`, `BanProvider` and `PluginMessaging`, and `Capability::ALL` has 20 entries.

What to do: replace `permission_level()` checks with `has_permission(ADMIN_PERMISSION)`, port checkers to `value`, register your nodes with defaults, and move per-player overrides to a provider if your plugin answers for everyone. See [Permissions](./permissions), in particular [Moving from permission levels](./permissions#moving-from-permission-levels).

## Bans

| 2.0.0-beta.3 `BanService` | Current `BanService` |
|---------------------------|----------------------|
| `ban(target, reason, duration) -> ()` | `ban(BanRequest) -> BanEntry` |
| `unban(&target) -> bool` | `unban(UnbanRequest) -> Option<BanEntry>` |
| `is_banned(&target) -> bool` | `get(&target)` returns the active ban on that target: `get(t).await?.is_some()` |
| `get_ban(&target) -> Option<BanEntry>` | `get(&target) -> Option<BanEntry>` |
| `get_all_bans() -> Vec<BanEntry>` | `list(BanQuery) -> BanPage` (cursor pagination), or `list_all()` |
| none | `check(&LoginAttempt) -> Option<BanVerdict>`: would this login be refused |
| none | `features()`: whether the provider supports IP ranges and pagination |

`BanEntry` gains a stable `id`, its `source` is a `BanSource` (`Console`, `Player`, `Plugin`, `WebApi`, `System`) instead of a `String`, it is `#[non_exhaustive]` and built with `BanEntry::new(id, target, source)` and its builder methods. `kick_message() -> String` became `default_kick_message() -> Component`. `BanTarget` gains `IpRange(IpNet)`.

```rust
// 2.0.0-beta.3
let bans = ctx.ban_service();
bans.ban(
    BanTarget::Username("Griefer".into()),
    Some("Griefing".into()),
    Some(Duration::from_secs(3600)),
)
.await
.map_err(|e| e.to_string())?;
let banned = bans
    .is_banned(&BanTarget::Username("Griefer".into()))
    .await
    .map_err(|e| e.to_string())?;

// current
let bans = ctx.ban_service_handle();
let entry = bans
    .ban(
        BanRequest::new(BanTarget::Username("Griefer".into()))
            .reason("Griefing")
            .duration(Duration::from_secs(3600)),
    )
    .await?;
tracing::info!("ban {} issued by {}", entry.id, entry.source);
let banned = bans
    .get(&BanTarget::Username("Griefer".into()))
    .await?
    .is_some();
```

**Compiles unchanged**: a ban now kicks every matching online player (by real IP, IP range, name or UUID) unless the request sets `kick(false)`, fires `BanIssuedEvent` or `BanRevokedEvent`, and each plugin's ban service records the plugin as the source. `ServiceError` converts into `PluginError`, so `?` works in `on_enable`.

The ban store itself is now replaceable: a plugin implements `BanProvider`, registers it with `ctx.register_ban_provider`, and the operator selects it with `[ban] provider`. Every check the proxy makes goes through the active provider, bounded by `[ban] check_timeout`, and fails closed.

What to do: build requests with `BanRequest` and `UnbanRequest`, replace `is_banned` and `get_all_bans`, and match `BanSource` instead of a string. See [Bans](./bans).

## Components and text

`Component` now models every content kind and the full style. Its fields changed; the builder did not.

| 2.0.0-beta.3 field | Current |
|--------------------|---------|
| `text: String` | `content: Content` (`Text`, `Translatable`, `Keybind`, `Score`, `Selector`, `Nbt`, `Object`); `as_text()` for the text of a `Text` component |
| `color: Option<String>` | `style.color: Option<TextColor>` (`Named(NamedColor)` or `Hex(u32)`) |
| `bold`, `italic`, `underlined`, `strikethrough`, `obfuscated` | `style.bold` and the others, or `style.decoration(Decoration)` |
| `extra: Vec<Component>` | `children: Vec<Component>` |
| `click_event: Option<ClickEvent>` | `style.click: Option<Box<ClickEvent>>` |
| `hover_event: Option<HoverEvent>` | `style.hover: Option<Box<HoverEvent>>` |

The builder methods (`text`, `error`, `color`, `bold` and the other decorations, `click`, `hover`, `append`, `join`) keep their names. `color` takes `impl IntoTextColor`: a `NamedColor`, a `TextColor`, or a `&str` or `String` that is a color name or `#rrggbb`. **Compiles unchanged**: a string that is neither leaves the color unset, where beta.3 stored any string. `append` takes `impl Into<Component>`, so `append("world")` works.

```rust
// 2.0.0-beta.3
let mut msg = Component::text("Hello ").color("gold").bold();
msg.extra.push(Component::text("world"));
let gold = msg.color.as_deref() == Some("gold");
let plain = msg.to_string();

// current
use infrarust_api::types::{NamedColor, TextColor};

let msg = Component::text("Hello ")
    .color(NamedColor::Gold)
    .bold()
    .append("world");
let gold = msg.style.color == Some(TextColor::Named(NamedColor::Gold));
let plain = msg.to_plain();
let json = msg.to_json_for(player.protocol_version());
```

`NamedColor`, `TextColor`, `Style`, `Content` and `Decoration` live in `infrarust_api::types`; the prelude does not re-export them. Other changes:

- Serialisers take the target protocol: `to_json_for(version)`, `to_json_value_for(version)`, `to_nbt_for(version)` and `to_legacy(code)`. `to_json()` and `to_nbt_network()` keep the beta.3 wire shape.
- Parsers: `from_json` accepts camelCase and snake_case, `from_nbt_network` and `from_legacy_with` are new, and all of them are depth-bounded and never panic. `ComponentParseError` gains `InvalidNbt`.
- `Display` prints the same text as `to_plain()`.
- `ClickEvent` gains `ChangePage` and `Custom`; `HoverEvent` gains `ShowItem` and `ShowEntity`, with `show_text`, `show_item` and `show_entity` constructors.
- **Compiles unchanged**: kick and denial reasons are serialised once, for the client's protocol and state. In beta.3 they were wrapped twice and players saw raw JSON. Messages, titles and action bars use the client's protocol, so click and hover events reach 1.21.5+ clients.

What to do: replace field access with `content`, `style` and `children`; use `NamedColor` constants instead of color strings where you can; send `Component` values and let the proxy serialise them.

## Player API

- `permission_level()` is removed. Use `has_permission(ADMIN_PERMISSION)`.
- `refresh_permissions()` is new.
- New methods, all with default implementations: `virtual_host`, `client_brand`, `settings`, `known_channels`, `ping`, `send_plugin_message`, `send_plugin_message_to_backend`, `connect`, `set_player_list_header_footer`, `clear_title`, `show_boss_bar`, `send_resource_pack`, `remove_resource_pack`, `transfer`, `store_cookie` and `request_cookie`.
- `connect(server)` resolves once the switch is over and returns a `ConnectionResult` (`Success`, `AlreadyConnected`, `Denied`, `Failed`, `Cancelled`); `switch_server` still returns once the session took the request.
- `PlayerError` gains `NoBackend`, `MessageTooLarge`, `Unsupported`, `InvalidArgument`, `Denied` and `WouldDeadlock`.
- **Compiles unchanged**: `remote_addr()` is the address from the PROXY protocol header when there is one, not the load balancer's. A kick sent with `disconnect` now reaches the client with its reason; in beta.3 about one in four arrived as a bare connection close.

```rust
// 2.0.0-beta.3
if player.permission_level() == PermissionLevel::Admin {
    let _ = player.switch_server(ServerId::new("lobby")).await;
}

// current
if player.has_permission(ADMIN_PERMISSION) {
    match player.connect(ServerId::new("lobby")).await {
        Ok(result) if result.is_success() => {}
        Ok(result) => tracing::info!("not moved: {}", result.as_str()),
        Err(e) => tracing::warn!("{e}"),
    }
}
```

What to do: replace `permission_level()`, and prefer `connect` where you need to know whether the switch worked. A command handler may await `connect` or `request_cookie` for the player who typed it. A listener of that player's session events or a limbo callback for that player gets `Err(PlayerError::WouldDeadlock)` at once: use `switch_server` there, or spawn the wait; see [Calling back into the proxy](./threading#calling-back-into-the-proxy). See [The Player trait](./api#the-player-trait).

## Services

| | 2.0.0-beta.3 | Current |
|-|--------------|---------|
| `register_limbo_handler(handler)` | `()`, only collected at startup | `Result<LimboHandlerRegistration, LimboHandlerError>`, at any time; `registration.unregister()` |
| `ServerManager::on_state_change(callback)` | Present, never removed on disable | Removed with `StateChangeCallback`: subscribe to `ServerStateChangeEvent` |
| Scheduler | `delay`, `interval`, `interval_with_delay`, `cancel` | Adds `spawn`, `delay_async`, `repeat` (runs never overlap) and `spawn_blocking`; tasks belong to the plugin |
| Sharing services between plugins | none | `ctx.services()`: `provide`, `get`, `provider`, withdrawn when the provider is disabled |
| `PlayerRegistry` | Name lookups case-sensitive | `get_player` is case-insensitive; `get_players_by_ip` is new |
| `PluginContext` handles | | `command_manager_handle`, `scheduler_handle`, `services_handle`, `channel_registrar`, `server_messenger`, `register_ban_provider`, `register_permission_provider`, `register_permission_node`, `permission_nodes` |

```rust
// 2.0.0-beta.3
ctx.register_limbo_handler(Box::new(Gate));
ctx.server_manager().on_state_change(Box::new(|server, old, new| {
    tracing::info!("{server} went from {old:?} to {new:?}");
}));

// current
let registration = ctx.register_limbo_handler(Box::new(Gate))?;
ctx.event_bus()
    .subscribe::<ServerStateChangeEvent, _>(EventPriority::NORMAL, |event| {
        tracing::info!(
            "{} went from {:?} to {:?}",
            event.server,
            event.old_state,
            event.new_state
        );
    });
```

`LimboHandlerError` converts into `PluginError`, so `?` works in `on_enable`. A limbo handler name belongs to the first plugin that registers it, and removing a handler releases the players it held.

**Compiles unchanged**: a task that panics no longer ends a repeating task, a plugin cannot cancel another plugin's task, and the plugin load order is stable instead of hash order.

What to do: handle the limbo registration result, move `on_state_change` callbacks to `ServerStateChangeEvent`, and use `repeat` or `spawn_blocking` for async or blocking periodic work. See [Plugin API](./api#scheduler), [Sharing Services](./services) and the [threading model](./threading).

## Filters

The codec and transport filter registries return a `Result` and know who owns each filter.

| | 2.0.0-beta.3 | Current |
|-|--------------|---------|
| `CodecFilterRegistry::register` | `()`; silently replaced another plugin's filter with the same id | `Result<(), FilterRegistryError>`; `OwnedBy { id, owner }` for another owner's id, re-registering your own id replaces it |
| `CodecFilterRegistry::unregister` | `()`; removed anyone's filter | `Result<(), FilterRegistryError>`; `NotFound` unless you own it |
| `TransportFilterRegistry` | Same as codec | Same changes |
| On disable | Filters stayed registered | The plugin's filters are removed |
| Transport chain | Snapshot taken at startup | Rebuilt on every change: a filter registered later runs, a removed one stops |

The proxy's own filters are owned by `PROXY_FILTER_OWNER` (`"infrarust"`).

```rust
// 2.0.0-beta.3
if let Some(codecs) = ctx.codec_filters() {
    codecs.register(Box::new(MyFilterFactory));
}

// current
if let Some(codecs) = ctx.codec_filters() {
    match codecs.register(Box::new(MyFilterFactory)) {
        Ok(()) => {}
        Err(FilterRegistryError::OwnedBy { id, owner }) => {
            tracing::warn!("{id} belongs to {owner}");
        }
        Err(e) => tracing::warn!("{e}"),
    }
}
```

What to do: handle the `Result` of `register` and `unregister`, and drop manual cleanup of filters in `on_disable`. See [Architecture & Pipeline](./architecture).

### Transport filters

`TransportFilter` is now a connection gate. In 2.0.0-beta.3 its data hooks were never called, `on_close` was never called, and `on_accept` ran inside the accept loop with an empty `real_ip` and a zero `connection_id`.

| | 2.0.0-beta.3 | Current |
|-|--------------|---------|
| `on_client_data`, `on_server_data` | Required, never called | Removed. There is no byte-stream access; use a codec filter or events |
| `FilterVerdict` | `Continue`, `Modified`, `Reject`, `#[non_exhaustive]` | `Continue`, `Reject`, exhaustive |
| `TransportContext::bytes_received`, `bytes_sent` | Always 0 | Removed |
| `on_accept` | Awaited in the accept loop, so a slow filter stalled every new connection | Runs in the connection's own task, after the PROXY protocol header is decoded |
| `real_ip` | Always `None` | The address from the PROXY protocol header, when enabled |
| `connection_id` | Always 0 | Non-zero, equal to the player's `PlayerId` for a login |
| `on_close` | Never called | Called once for every filter that returned `Continue`, in reverse order, however the connection ended; not for the rejecting filter or the ones after it |
| A panic or a slow `on_accept` | Panic killed the accept loop; no bound | Rejects the connection (fail closed) after `[events] transport_filter_timeout`, with a warning naming the filter and its plugin |
| Rejection | Not reported | Posts `ConnectionRejectedEvent` with `RejectReason::Plugin { plugin_id }` |

```rust
// 2.0.0-beta.3
impl TransportFilter for Gate {
    fn metadata(&self) -> FilterMetadata { FilterMetadata::new("gate") }
    fn on_accept<'a>(&'a self, ctx: &'a mut TransportContext) -> BoxFuture<'a, FilterVerdict> {
        Box::pin(async move { FilterVerdict::Continue })
    }
    fn on_client_data<'a>(&'a self, _: &'a mut TransportContext, _: &'a mut BytesMut) -> BoxFuture<'a, FilterVerdict> {
        Box::pin(async { FilterVerdict::Continue })
    }
    fn on_server_data<'a>(&'a self, _: &'a mut TransportContext, _: &'a mut BytesMut) -> BoxFuture<'a, FilterVerdict> {
        Box::pin(async { FilterVerdict::Continue })
    }
}

// current
impl TransportFilter for Gate {
    fn metadata(&self) -> FilterMetadata { FilterMetadata::new("gate") }
    fn on_accept<'a>(&'a self, ctx: &'a mut TransportContext) -> BoxFuture<'a, FilterVerdict> {
        Box::pin(async move { FilterVerdict::Continue })
    }
    fn on_close(&self, ctx: &TransportContext) {}
}
```

What to do: delete the two data hooks and any use of the byte counters or `FilterVerdict::Modified`, and move per-connection cleanup into `on_close`, which now runs. See [Architecture & Pipeline](./architecture#layer-1-transportfilter).

## Plugin messaging

Plugin messaging is new. A plugin registers channels with `ctx.channel_registrar()`, receives `PluginMessageEvent` for them in either direction, sends with `Player::send_plugin_message` and `send_plugin_message_to_backend`, and reaches a server with `ctx.server_messenger()`. The proxy also keeps the client's brand, settings and channels across server switches.

Two changes affect existing setups:

- **Compiles unchanged**: a client can no longer send a message on `BungeeCord`, `bungeecord:main` or a `velocity:` channel. The proxy drops it, since backend plugins trust those channels to come from the proxy.
- The BungeeCord channel is now implemented by the proxy, off by default. The beta.3 keys `[forwarding] bungeecord_channel` and `[forwarding.channel_permissions]` are ignored with a warning; see [Configuration keys](#configuration-keys).

See [Plugin Messaging](./messaging).

## Configuration keys

| Key | Default | Change |
|-----|---------|--------|
| `[events] handler_timeout` | `10s` | New: an async listener still running is cancelled |
| `[events] slow_handler_threshold` | `1s` | New: a listener slower than this is logged |
| `[events] packet_handler_timeout` | `10s` | New: the same limit for raw packet listeners |
| `[events] disconnect_deadline` | `15s` | New: bounds all `DisconnectEvent` listeners of one player, and the wait for a displaced session |
| `[ban] provider` | `"builtin"` | New: `"builtin"`, `"none"` or the id of the plugin whose `BanProvider` answers |
| `[ban] check_timeout` | `5s` | New: bounds each ban check; a timeout refuses the login |
| `[permissions] provider` | `"builtin"` | New: `"builtin"` or the id of the plugin whose `PermissionProvider` answers |
| `[permissions] trust_offline_admins` | `false` | New: an offline-mode player listed in `admins` is an admin only when `true` |
| `[plugin_messaging] bungeecord` | `false` | New: the proxy answers the BungeeCord channel |
| `[plugin_messaging.bungeecord_permissions]` | | New: which BungeeCord subchannels are allowed |
| `bungeecord_channel` in a server file | `false` | New: the per-server opt-in for the BungeeCord channel |
| `[forwarding] bungeecord_channel`, `[forwarding.channel_permissions]` | | Removed: ignored with a warning |
| `[events] transport_filter_timeout` | `5s` | New: bounds each transport filter's `on_accept`; a timeout rejects the connection |
| `[auth] offline_uuid` | `"offline"` | New: see [Offline UUIDs](#offline-uuids) |

`[wasm]`, `[plugins.<id>.wasm]` (including `wasm.network` and `wasm.mounts`), `[plugins.<id>] deny` and `[plugins.<id>] strict_capabilities` only apply to WASM plugins. Every key is described in [Global Settings](../../configuration/global).

## Offline UUIDs

**Compiles unchanged**, and changes the identity of offline players.

In 2.0.0-beta.3, a player the session server does not verify (every player on an `offline` or passthrough server, and a `client_only` player let in with `ForceOffline`) got the UUID the client sent in its login start packet, or a new random UUID on every connection when it sent none. `ForceOffline` fell back to a name-based UUID, but computed it with a nil namespace, so it differed from the one an offline backend computes.

Now those players get the vanilla offline UUID, the one Java's `UUID.nameUUIDFromBytes("OfflinePlayer:" + name)` returns: the same name always gets the same UUID, and it matches the backend's. `Notch` is `b50ad385-829d-3141-a216-7e7d7539ba7f`. A plugin can still set any UUID in `GameProfileRequestEvent`.

To keep the UUID the client claims, set:

```toml
[auth]
offline_uuid = "client"
```

With `"client"`, the proxy uses the UUID from the login start packet when the client sends one, and the name-based offline UUID otherwise. It never picks a random one.

What to do: if your plugin stored data by the UUIDs of offline players under beta.3 (bans by UUID, economy balances, statistics), either migrate those records to the name-based UUIDs or set `offline_uuid = "client"`. See [Authentication](../../configuration/global#authentication).

## Checklist

- [ ] Bump `infrarust-api` and fix the build.
- [ ] Replace `event.player_id` with `event.player_id()` and use `event.player` instead of registry lookups.
- [ ] Build events in tests with `new` and `MockPlayer` from the `test-util` feature.
- [ ] Replace `ServerSwitchEvent` with `ServerPostConnectEvent` and `switched_from()`.
- [ ] Rename `ServerPreConnectEvent::original_server` to `server` and drop `ServerPreConnectResult::VirtualBackend`.
- [ ] Check `ServerConnectedEvent` handlers against the new timing.
- [ ] Handle `KickedFromServerEvent::reason` as an `Option` and stop assuming the default result is a disconnect.
- [ ] Use `DisconnectEvent::cause`, `username()` and `player_id()`, and remove ghost-player workarounds.
- [ ] Keep `PostLoginEvent` listeners fast: the join now waits for them.
- [ ] Rename `ChatMessageResult::Modify { new_message }` to `{ message }` and wrap `Deny` reasons in `Some`.
- [ ] Move `RawPacketEvent` listeners to `subscribe_packet_typed`.
- [ ] Handle the `bool` from `unsubscribe` and only unsubscribe your own handles.
- [ ] Port commands to `CommandSpec`, `CommandContext::source` and `suggest`, and handle `CommandError`.
- [ ] Port permission checkers to `value(node) -> Tristate`, replace `permission_level()` with `ADMIN_PERMISSION`, and register your nodes.
- [ ] Port ban calls to `BanRequest`, `UnbanRequest`, `get`, `list` and `BanSource`.
- [ ] Replace `Component` field access with `content`, `style` and `children`, and check color strings.
- [ ] Handle the result of `register_limbo_handler`.
- [ ] Replace `ServerManager::on_state_change` with `ServerStateChangeEvent`.
- [ ] Handle `FilterRegistryError` from the codec and transport registries.
- [ ] Move `[forwarding] bungeecord_channel` to `[plugin_messaging]` and the server files.
- [ ] Review `[events]`, `[ban]` and `[permissions]` defaults for your deployment.
- [ ] Decide what to do with data keyed by offline UUIDs, or set `[auth] offline_uuid = "client"`.

## See also

- [Events Reference](./events): every event, its fields, results and delivery.
- [Threading Model](./threading): where handlers run and what they may block.
- [Commands](./commands), [Permissions](./permissions), [Bans](./bans), [Plugin Messaging](./messaging), [Sharing Services](./services).
- [Testing Plugins](./testing): mocks from the `test-util` feature.
- [Migrating to 0.3](../wasm/migration-0.3): the WASM counterpart of this page.
