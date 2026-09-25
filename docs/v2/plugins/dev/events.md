---
title: Events Reference
description: Complete reference of all proxy events, their payloads, results, and usage examples.
outline: [2, 3]
---

# Events Reference

Infrarust fires events at key points in a player's lifecycle, from initial connection through disconnect. Your plugin subscribes to these events through the `EventBus`, and for resulted events, you can modify the outcome.

## Player lifecycle

The events a player goes through depend on the proxy mode of the server they join.

### `offline` and `client_only`

The proxy runs the login itself, so every step can be observed and most can be refused:

```
PreLoginEvent ─────────────── Denied ──▶ disconnected during login
  → authentication (client_only: encryption and session server)
                            ─ failed ──▶ OnlineAuthFailed, disconnected
  → GameProfileRequestEvent    the profile can be rewritten
  → ban check (IP, name, final UUID) ─ banned ──▶ disconnected during login
  → PermissionsSetupEvent
  → LoginEvent ──────────────── Denied ──▶ disconnected during login
  → client_only: LoginSuccess sent with the final profile
  → player registered
  → PostLoginEvent
  → PlayerChooseInitialServerEvent → ServerPreConnectEvent
  → backend connection → ServerConnectedEvent
  → play (ChatMessageEvent, raw packets, switches: ServerPreConnectEvent → ServerSwitchEvent)
  → DisconnectEvent
```

### Passthrough, `zero_copy` and `server_only`

The backend runs the login and the proxy only forwards bytes:

```
player registered → PostLoginEvent → ServerPreConnectEvent
  → backend connection → ServerConnectedEvent → forwarding → DisconnectEvent
```

These modes do not fire `PreLoginEvent`, `GameProfileRequestEvent`, `PermissionsSetupEvent` or `LoginEvent` yet. Clients older than 1.7 (the legacy protocol) fire no player events.

### Guarantees

- Every event up to and including `PostLoginEvent` is awaited: the login waits for all listeners before it moves on.
- A login that ends before `PostLoginEvent` (denied, banned, failed authentication) never creates a player, so no `DisconnectEvent` follows.
- Once `PostLoginEvent` has fired, `DisconnectEvent` fires exactly once for that player, whatever ends the session: the client leaving, a kick, a denied or failed initial connection, the backend closing, a proxy shutdown or an error.
- During `PostLoginEvent` the player is already in the player registry (`get_player_by_id`, `get_player` and `get_player_by_uuid` find it) and `current_server()` is `None`, because the player has not been routed yet.
- A `player.disconnect(reason)` made during `PostLoginEvent` disconnects the client in the state it is in (the login phase for `offline`, before any server is chosen) with that reason, and the player's `DisconnectEvent` follows. Messages, titles and action bars sent during `PostLoginEvent` wait and reach the client once it has joined the game.
- When a player logs in with a UUID that is already online, the proxy disconnects the first session with "You logged in from another location" and waits for its `DisconnectEvent` before the new session's `PostLoginEvent`, for at most `[events] disconnect_deadline`.
- `DisconnectEvent` is awaited, and the player leaves the registry once its listeners are done. The whole dispatch is bounded by [`disconnect_deadline`](../../configuration/global#plugin-event-handlers) (15 seconds by default): listeners still running then are cancelled, and the player is removed anyway.

## Subscribing to events

Get the event bus from your `PluginContext` in `on_enable`:

```rust
use infrarust_api::prelude::*;

ctx.event_bus().subscribe::<PostLoginEvent, _>(
    EventPriority::NORMAL,
    |event| {
        tracing::info!("Player joined: {}", event.profile.username);
    },
);
```

For async work, use `subscribe_async`:

```rust
ctx.event_bus().subscribe_async::<DisconnectEvent, _>(
    EventPriority::NORMAL,
    |event| Box::pin(async move {
        save_player_data(event.player_id()).await;
    }),
);
```

Both methods return a `ListenerHandle` you can pass to `event_bus().unsubscribe(handle)` to remove the listener. `unsubscribe` returns `true` when it removed something. A plugin can only remove its own listeners: passing a handle that another plugin registered does nothing, returns `false`, and logs a warning.

## When a listener fails

A listener that panics is skipped: the proxy logs the panic with your plugin ID and passes the event on to the next listener. Anything the listener changed on the event before panicking is kept, so set the result last if a later step might fail.

When the failing listener was handling an event that another plugin fired (see [custom events](#custom-events)), the log line and the handler diagnostic also carry `fired_by`, the ID of the plugin that fired it. For events the proxy fires, `fired_by` is `infrarust`.

Async listeners also have a time limit, `handler_timeout` in the [`[events]`](../../configuration/global#plugin-event-handlers) section (10 seconds by default, `packet_handler_timeout` for raw packet listeners). A listener still running at the deadline is cancelled and the event continues without it. A listener that takes longer than `slow_handler_threshold` (1 second by default) is logged as slow. Synchronous listeners can't be interrupted, so keep blocking work out of them.

## Priority

Listeners run in priority order. Lower values run first.

| Constant | Value | When to use |
|----------|-------|-------------|
| `FIRST`  | 0     | Security checks, logging that must see the original event |
| `EARLY`  | 64    | Validation before normal processing |
| `NORMAL` | 128   | Default. Most plugins use this |
| `LATE`   | 192   | React to decisions made by earlier listeners |
| `LAST`   | 255   | Final overrides, monitoring |

Each listener sees modifications made by previous listeners. Use `EventPriority::custom(u8)` for fine-grained control.

## Resulted vs informational events

Some events implement `ResultedEvent`. These have a result that controls what the proxy does next. Call `event.set_result()` to change the outcome, or use shortcut methods like `event.deny()`.

Informational events (like `PostLoginEvent`) have no result. You can read their fields and act on the player they carry, but you cannot change what the proxy does next through them.

## Delivery

Every event goes through the same dispatch: listeners run one after another in priority order, with the panic isolation and timeouts described above. What differs is when that dispatch happens relative to the code that fired the event.

| Delivery | Events | What it means |
|----------|--------|---------------|
| Inline, awaited | `PreLoginEvent`, `OnlineAuthFailed`, `GameProfileRequestEvent`, `PermissionsSetupEvent`, `LoginEvent`, `PostLoginEvent`, `PlayerChooseInitialServerEvent`, `ServerPreConnectEvent`, `KickedFromServerEvent`, `ChatMessageEvent`, `ProxyPingEvent`, `ProxyInitializeEvent`, `ProxyShutdownEvent`, `DisconnectEvent`, custom events | The proxy (or the plugin that fired it) waits for every listener before it continues, so listeners can change the outcome. `DisconnectEvent` is also bounded as a whole by `[events] disconnect_deadline`. |
| Detached, per player | `ServerConnectedEvent`, `ServerSwitchEvent` | Each event is dispatched in its own task. The player's connection does not wait for it, and there is no ordering guarantee between two of these events. |
| Queued, in order | `ServerStateChangeEvent`, `BackendHealthEvent`, `ConfigReloadEvent` | The proxy posts these to a single queue. One dispatcher delivers them in the order they were posted, one event at a time. |

Because the queue delivers one event at a time, a slow listener on a queued event delays the queued events behind it, up to `handler_timeout` per listener. A listener that panics does not stop the queue: the next event is still delivered.

## Lifecycle events

### PreLoginEvent

Fired before authentication, when a player initiates a connection. This is your first chance to accept, deny, or change the auth mode for a player.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `profile` | `GameProfile` | The player's profile. In `client_only` the UUID is nil until authentication; offline profiles follow [`[auth] offline_uuid`](../../configuration/global#authentication) |
| `remote_addr` | `SocketAddr` | The client's address. Behind a load balancer that sends the PROXY protocol ([`receive_proxy_protocol`](../../configuration/global#proxy-protocol)), the address from the header |
| `protocol_version` | `ProtocolVersion` | Protocol version reported by the client |
| `server_domain` | `String` | The domain from the handshake packet |

**Results** (`PreLoginResult`):

| Variant | Description |
|---------|-------------|
| `Allowed` (default) | Proceed with normal authentication |
| `Denied { reason }` | Kick the player with a message |
| `ForceOffline` | Skip Mojang auth for this player |
| `ForceOnline` | Force Mojang auth even if the server is in offline mode |

```rust
ctx.event_bus().subscribe::<PreLoginEvent, _>(
    EventPriority::NORMAL,
    |event| {
        // Ban check
        if is_banned(&event.profile.uuid) {
            event.deny(Component::error("You are banned."));
        }
    },
);
```

### OnlineAuthFailed

Fired when online-mode authentication fails, for example a cracked client that cannot complete the encryption handshake. This covers both forced online auth (`ForceOnline` returned from `PreLoginEvent` while the server is in offline mode) and the default online auth used by `client_only` mode. A plugin can listen for this to remember the username and return `ForceOffline` on the player's next connection attempt. Informational only, and awaited: the client is disconnected once the listeners are done.

| Field | Type | Description |
|-------|------|-------------|
| `username` | `String` | The username that failed online authentication |

`OnlineAuthFailed` is not re-exported from the prelude. Import it from its module: `use infrarust_api::events::lifecycle::OnlineAuthFailed;`.

### GameProfileRequestEvent

Fired right after authentication, before the proxy checks bans against the player's UUID and before the player exists. Change `profile` to give the player another UUID, name or properties, for example skin textures in offline mode. The profile left in the event when the last listener returns is the one the proxy uses from then on: for the UUID ban check, in the player registry and every later event, in the `LoginSuccess` the client receives, and in what forwarding sends to the backend.

**Type:** Informational, with a mutable `profile`

| Field | Type | Description |
|-------|------|-------------|
| `profile` | `GameProfile` | The profile the player will have. Change it to rewrite the player's identity |
| `online_mode` | `bool` | Whether the session server verified the player |
| `remote_addr` | `SocketAddr` | The client's address |
| `virtual_host` | `Option<String>` | The domain from the handshake |
| `protocol_version` | `ProtocolVersion` | The client's protocol version |

`original()` returns the profile as authentication produced it, and `is_modified()` tells whether a listener changed it.

```rust
use infrarust_api::events::lifecycle::GameProfileRequestEvent;

ctx.event_bus().subscribe::<GameProfileRequestEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if !event.online_mode {
            event.profile.properties = skin_for(&event.profile.username);
        }
    },
);
```

In `offline` mode the backend normally completes the login with the client. When a listener changes the profile, the proxy completes the login itself instead, as it does for Velocity forwarding, so that the client and the backend both see the new profile.

### PermissionsSetupEvent

Fired after the ban check, before `LoginEvent`. This is the extension point for replacing the default permission checker with one backed by LuckPerms, a database, or any external permission system. If no listener provides a custom checker, the proxy keeps its built-in `ConfigPermissionChecker`, which reads admin UUIDs from `[permissions].admins`. See [permissions](../../configuration/security/permissions.md) for the two-level model (Player and Admin).

The player is built but not registered yet. The checker a listener sets applies to the player from then on, so `has_permission` already answers with it in `LoginEvent` and `PostLoginEvent`.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player being logged in |
| `online_mode` | `bool` | Whether the player authenticated via Mojang |

`player_id()` and `profile()` are shortcuts for `player.id()` and `player.profile()`.

**Results** (`PermissionsSetupResult`):

| Variant | Description |
|---------|-------------|
| `UseDefault` (default) | Use the proxy's built-in config-based checker |
| `Custom(Arc<dyn PermissionChecker>)` | Use a plugin-provided checker |

```rust
use infrarust_api::events::lifecycle::{PermissionsSetupEvent, PermissionsSetupResult};

ctx.event_bus().subscribe::<PermissionsSetupEvent, _>(
    EventPriority::NORMAL,
    |event| {
        event.set_result(PermissionsSetupResult::Custom(
            Arc::new(MyPermissionChecker::new()),
        ));
    },
);
```

Like `OnlineAuthFailed`, this type is reached through `infrarust_api::events::lifecycle`, not the prelude glob.

### LoginEvent

Fired after `PermissionsSetupEvent`, just before the player is registered. It is the last point where a login can be refused: a denied player is disconnected during the login phase with the reason, is never registered, and fires neither `PostLoginEvent` nor `DisconnectEvent`. In `client_only` mode the client has not received `LoginSuccess` yet.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player being logged in |
| `online_mode` | `bool` | Whether the player authenticated via Mojang |

`player_id()` and `profile()` are shortcuts for `player.id()` and `player.profile()`.

**Results** (`LoginResult`):

| Variant | Description |
|---------|-------------|
| `Allowed` (default) | Let the player in |
| `Denied { reason }` | Disconnect the player during login with this reason |

```rust
ctx.event_bus().subscribe::<LoginEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if !is_whitelisted(&event.profile().uuid) {
            event.deny(Component::error("You are not whitelisted."));
        }
    },
);
```

### PostLoginEvent

Fired once the player is registered, before the proxy picks their first server. Informational and awaited: the proxy routes the player only after every listener has returned. See the [guarantees](#guarantees) for what a listener can rely on here.

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player, already in the player registry |
| `profile` | `GameProfile` | The player's final profile |
| `protocol_version` | `ProtocolVersion` | The player's protocol version |

`player_id()` is a shortcut for `player.id()`.

```rust
ctx.event_bus().subscribe::<PostLoginEvent, _>(
    EventPriority::NORMAL,
    |event| {
        tracing::info!("{} logged in ({:?})", event.profile.username, event.player_id());
        let _ = event.player.send_message(Component::text("Welcome!"));
    },
);
```

### DisconnectEvent

Fired exactly once for every player that got a `PostLoginEvent`, when their session ends. The proxy waits for all listeners, up to `[events] disconnect_deadline`, then removes the player from the registry, so async cleanup can still look the player up.

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The disconnecting player |
| `last_server` | `Option<ServerId>` | The server they were on when the session ended, `None` if they never reached one |
| `cause` | `DisconnectCause` | Why the session ended |

`player_id()` and `username()` are shortcuts for `player.id()` and `player.profile().username`.

**`DisconnectCause`** (`as_str()` gives the name in parentheses, `reason()` the message when there is one):

| Variant | Description |
|---------|-------------|
| `ClientQuit` (`client_quit`) | The client closed the connection |
| `Kicked { reason }` (`kicked`) | The proxy ended the session: `Player::disconnect`, a denied connection, a duplicate login. `reason` is what the client was shown |
| `BackendClosed { reason }` (`backend_closed`) | The backend kicked the player or closed the connection and the player was not moved elsewhere |
| `Shutdown` (`shutdown`) | The proxy is shutting down |
| `Error` (`error`) | The session failed, for example an I/O error or an initial backend that could not be reached |

```rust
ctx.event_bus().subscribe_async::<DisconnectEvent, _>(
    EventPriority::NORMAL,
    |event| Box::pin(async move {
        tracing::info!("{} disconnected ({})", event.username(), event.cause.as_str());
    }),
);
```

### WASM plugins

WASM plugins (contract 0.2.3) receive `PreLoginEvent`, `OnlineAuthFailed`, `PermissionsSetupEvent`, `PostLoginEvent` and `DisconnectEvent` with the same fields as before, in the order above. `GameProfileRequestEvent` and `LoginEvent` are not delivered to WASM plugins yet, and the WASM `DisconnectEvent` has no cause.

## Connection events

### PlayerChooseInitialServerEvent

Fired after `PostLoginEvent`, before `ServerPreConnectEvent`. Allows you to override which server a player connects to first. Useful for lobby systems, load balancing, or queue plugins.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player_id` | `PlayerId` | The connecting player |
| `profile` | `GameProfile` | The player's profile |
| `initial_server` | `ServerId` | The server chosen by the domain router |

**Results** (`PlayerChooseInitialServerResult`):

| Variant | Description |
|---------|-------------|
| `Allowed` (default) | Use the domain router's choice |
| `Redirect(ServerId)` | Send to a different server |
| `SendToLimbo { limbo_handlers }` | Route through limbo handlers |

```rust
ctx.event_bus().subscribe::<PlayerChooseInitialServerEvent, _>(
    EventPriority::NORMAL,
    |event| {
        // Send new players to the lobby
        if is_first_join(&event.profile.uuid) {
            event.set_result(PlayerChooseInitialServerResult::Redirect(
                ServerId::new("lobby"),
            ));
        }
    },
);
```

### ServerPreConnectEvent

Fired before the proxy connects a player to a backend server. This fires on initial connection and on every server switch.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player_id` | `PlayerId` | The player |
| `profile` | `GameProfile` | The player's profile |
| `original_server` | `ServerId` | The target server |

**Results** (`ServerPreConnectResult`):

| Variant | Description |
|---------|-------------|
| `Allowed` (default) | Connect to the original server |
| `ConnectTo(ServerId)` | Redirect to a different server |
| `SendToLimbo { limbo_handlers }` | Route through limbo handlers |
| `VirtualBackend(Box<dyn VirtualBackendHandler>)` | Route to a virtual backend handler |
| `Denied { reason }` | Block the connection |

```rust
ctx.event_bus().subscribe::<ServerPreConnectEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if is_server_full(&event.original_server) {
            event.redirect_to(ServerId::new("fallback"));
        }
    },
);
```

### ServerConnectedEvent

Fired after a player has connected to a backend server. Informational.

| Field | Type | Description |
|-------|------|-------------|
| `player_id` | `PlayerId` | The player |
| `server` | `ServerId` | The server they connected to |

### ServerSwitchEvent

Fired after a player switches from one server to another. Informational.

| Field | Type | Description |
|-------|------|-------------|
| `player_id` | `PlayerId` | The player |
| `previous_server` | `ServerId` | The server they left |
| `new_server` | `ServerId` | The server they moved to |

### KickedFromServerEvent

Fired when a backend server kicks a player. You can decide what happens next: disconnect them, redirect them, send them to limbo, or just show a message.

**Type:** Resulted (default: `DisconnectPlayer`)

| Field | Type | Description |
|-------|------|-------------|
| `player_id` | `PlayerId` | The kicked player |
| `server` | `ServerId` | The server that kicked them |
| `reason` | `Component` | The kick reason from the server |

**Results** (`KickedFromServerResult`):

| Variant | Description |
|---------|-------------|
| `DisconnectPlayer { reason }` (default) | Disconnect from the proxy |
| `RedirectTo(ServerId)` | Send to another server |
| `SendToLimbo { limbo_handlers }` | Route through limbo handlers |
| `Notify { message }` | Keep connected, show a message |

```rust
ctx.event_bus().subscribe::<KickedFromServerEvent, _>(
    EventPriority::NORMAL,
    |event| {
        // If kicked from a game server, send to lobby instead of disconnecting
        event.redirect_to(ServerId::new("lobby"));
    },
);
```

## Chat events

### ChatMessageEvent

Fired when a player sends a chat message during Play state.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player_id` | `PlayerId` | The sender |
| `message` | `String` | The message text |

**Results** (`ChatMessageResult`):

| Variant | Description |
|---------|-------------|
| `Allow` (default) | Forward the message |
| `Deny { reason }` | Block the message, show a reason to the sender |
| `Modify { new_message }` | Replace the message text |

```rust
ctx.event_bus().subscribe::<ChatMessageEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if contains_banned_word(&event.message) {
            event.deny(Component::error("That word is not allowed."));
        }
    },
);
```

## Packet events

### RawPacketEvent

A low-level event fired when a raw packet passes through the proxy during Play state. Use this only when higher-level events don't cover your use case.

`RawPacketEvent` does not implement `Event` or `ResultedEvent`, so `subscribe::<RawPacketEvent, _>` does not compile. The only way to receive raw packets is a packet subscription (`subscribe_packet_typed` or `subscribe_packet_async_typed`, shown below). Read and change the outcome with the event's own `result()`, `set_result()`, `drop_packet()` and `modify()` methods.

WASM plugins cannot receive raw packets in the current contract version. A WASM plugin that subscribes to `raw-packet` gets a listener ID back, but no listener is registered and the proxy logs a warning naming the plugin.

| Field | Type | Description |
|-------|------|-------------|
| `player_id` | `PlayerId` | The player this packet belongs to |
| `direction` | `PacketDirection` | `Serverbound` (client to server) or `Clientbound` (server to client) |
| `packet` | `RawPacket` | The raw packet data |

**Results** (`RawPacketResult`):

| Variant | Description |
|---------|-------------|
| `Pass` (default) | Forward unmodified |
| `Modify { packet }` | Replace with a different packet |
| `Drop` | Silently discard the packet |

Packet events use a different subscription API. Instead of subscribing to all packets, you register a `PacketFilter` for the specific packet ID, connection state, and direction you care about:

```rust
use infrarust_api::event::{PacketFilter, ConnectionState, PacketDirection};

let filter = PacketFilter {
    packet_id: 0x03,
    state: ConnectionState::Play,
    direction: PacketDirection::Serverbound,
};

ctx.event_bus().subscribe_packet_typed(
    filter,
    EventPriority::NORMAL,
    |event| {
        tracing::debug!("Received packet 0x03 from {:?}", event.player_id);
    },
);
```

The proxy skips event dispatch for packets with no registered listeners, so this is efficient even at high packet rates.

::: warning
Packet events run on every matching packet in the forwarding loop. Keep handlers fast to avoid adding latency.
:::

## Proxy events

These events relate to the proxy itself rather than individual players.

### ProxyPingEvent

Fired when a client pings the server list. You can modify the response to customize the MOTD, player count, version, and favicon.

The `ProxyPingEvent` does not implement `ResultedEvent`. Instead, mutate the `response` field directly.

| Field | Type | Description |
|-------|------|-------------|
| `remote_addr` | `SocketAddr` | The pinging client's address |
| `response` | `PingResponse` | The response to send back (mutable) |

`PingResponse` fields:

| Field | Type | Description |
|-------|------|-------------|
| `description` | `Component` | The MOTD shown in the server list |
| `max_players` | `i32` | Maximum player count |
| `online_players` | `i32` | Current online player count |
| `protocol_version` | `ProtocolVersion` | The protocol version to report |
| `version_name` | `String` | Version name string (e.g. "Infrarust 2.0") |
| `favicon` | `Option<String>` | Base64-encoded 64x64 PNG, if any |

```rust
ctx.event_bus().subscribe::<ProxyPingEvent, _>(
    EventPriority::NORMAL,
    |event| {
        let resp = event.response_mut();
        resp.description = Component::text("My Minecraft Network").color("gold");
        resp.max_players = 500;
    },
);
```

### ProxyInitializeEvent

Fired after the proxy finishes startup and all plugins are loaded. No fields. Use this for setup that depends on other plugins being ready.

### ProxyShutdownEvent

Fired when the proxy shuts down. No fields. Use this or `Plugin::on_disable` for resource cleanup.

### ConfigReloadEvent

Fired when the proxy configuration is hot-reloaded. No fields. Subscribe to this to re-read your plugin's config at runtime.

```rust
ctx.event_bus().subscribe::<ConfigReloadEvent, _>(
    EventPriority::NORMAL,
    |_event| {
        tracing::info!("Config reloaded, refreshing plugin settings");
    },
);
```

### ServerStateChangeEvent

Fired when a backend server changes state (online, offline, starting, stopping, sleeping, crashed).

| Field | Type | Description |
|-------|------|-------------|
| `server` | `ServerId` | The server whose state changed |
| `old_state` | `ServerState` | Previous state |
| `new_state` | `ServerState` | New state |

`ServerState` variants: `Online`, `Offline`, `Starting`, `Stopping`, `Sleeping`, `Crashed`.

```rust
ctx.event_bus().subscribe::<ServerStateChangeEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if matches!(event.new_state, ServerState::Crashed) {
            tracing::error!("Server {:?} crashed!", event.server);
        }
    },
);
```

### BackendHealthEvent

Fired when a backend address changes health state, for example when it stops accepting connections or recovers. Informational, delivered through the ordered queue.

| Field | Type | Description |
|-------|------|-------------|
| `address` | `ServerAddress` | The backend address whose health changed |
| `servers` | `Vec<ServerId>` | The servers that list this address |
| `state` | `BackendState` | The new health state |

## Custom events

Plugins can define and fire their own events. Any type that implements `Event` works: listeners subscribe to it with `subscribe` or `subscribe_async` as usual, and the firing plugin calls `fire` on its event bus.

```rust
use infrarust_api::prelude::*;

pub struct PartyInvite {
    pub from: String,
    pub to: String,
    pub accepted: Option<bool>,
}

impl Event for PartyInvite {}
```

`fire` dispatches the event inline and returns it once every listener has run, so listeners can answer by writing into it:

```rust
ctx.event_bus().subscribe::<PartyInvite, _>(EventPriority::NORMAL, |invite| {
    invite.accepted = Some(invite.to != "Mallory");
});

let bus = ctx.event_bus_handle();
let invite = bus
    .fire(PartyInvite {
        from: "Steve".into(),
        to: "Alex".into(),
        accepted: None,
    })
    .await?;
```

`fire` returns `Result<E, FireError>`. The event types defined by `infrarust_api::events` belong to the proxy: firing one of them from a plugin returns `Err(FireError::Reserved)` and no listener runs, so a plugin cannot forge a `PreLoginEvent` or a `ConfigReloadEvent`. `NamedEvent` is the one exception, since it exists for plugins to fire.

Two plugins exchange a custom event only if they use the same Rust type. Put the event types in a crate both plugins depend on.

To fire from inside a listener, keep the `Arc<dyn EventBus>` returned by `ctx.event_bus_handle()` in the listener and call `fire` on it. A listener that fires an event waits for that event's listeners, and the time counts against its own `handler_timeout`.

### NamedEvent

`NamedEvent` is a general-purpose event identified by a string name, with an opaque payload. It needs no shared Rust type, only an agreement on the name and the payload format.

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | The message name listeners filter on |
| `source_plugin` | `String` | The ID of the plugin that fired the event |
| `content_type` | `String` | The payload format, for example `application/json` |
| `payload` | `Bytes` | The message body |
| `cancelled` | `bool` | Set by a listener through `cancel()` |
| `response` | `Option<NamedEventResponse>` | Set by a listener through `respond(content_type, payload)` |

Create one with `NamedEvent::new(name, content_type, payload)`. The proxy overwrites `source_plugin` with the ID of the firing plugin, so listeners can trust it. Every `NamedEvent` listener receives every named event, so check `name` first:

```rust
use infrarust_api::events::NamedEvent;

ctx.event_bus().subscribe::<NamedEvent, _>(EventPriority::NORMAL, |event| {
    if event.name != "economy:balance" {
        return;
    }
    event.respond("text/plain", "42");
});

let answered = ctx
    .event_bus_handle()
    .fire(NamedEvent::new("economy:balance", "text/plain", "Steve"))
    .await?;
```

WASM plugins cannot fire or subscribe to custom events or `NamedEvent` in the current contract version.
