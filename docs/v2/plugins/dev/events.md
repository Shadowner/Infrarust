---
title: Events Reference
description: Complete reference of all proxy events, their payloads, results, and usage examples.
outline: [2, 3]
---

# Events Reference

Infrarust fires events at key points in a player's lifecycle, from initial connection through disconnect. Your plugin subscribes to these events through the `EventBus`, and for resulted events, you can modify the outcome.

## Event flow

A player connection follows this path:

```
PreLoginEvent → Authentication → PostLoginEvent
    → PlayerChooseInitialServerEvent → ServerPreConnectEvent
    → Backend connection → ServerConnectedEvent
    → Play state (ChatMessageEvent, RawPacketEvent)
    → Server switch → ServerPreConnectEvent → ServerSwitchEvent
    → DisconnectEvent
```

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
        save_player_data(event.player_id).await;
    }),
);
```

Both methods return a `ListenerHandle` you can pass to `event_bus().unsubscribe(handle)` to remove the listener.

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

Informational events (like `PostLoginEvent`) are fire-and-forget. You can read their fields but cannot change the proxy's behavior through them.

## Lifecycle events

### PreLoginEvent

Fired before authentication, when a player initiates a connection. This is your first chance to accept, deny, or change the auth mode for a player.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `profile` | `GameProfile` | The player's profile (may be incomplete in offline mode) |
| `remote_addr` | `SocketAddr` | The connecting client's address |
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

### PostLoginEvent

Fired after a player has successfully authenticated. Informational only.

| Field | Type | Description |
|-------|------|-------------|
| `profile` | `GameProfile` | The authenticated profile |
| `player_id` | `PlayerId` | The player's session ID |
| `protocol_version` | `ProtocolVersion` | The player's protocol version |

```rust
ctx.event_bus().subscribe::<PostLoginEvent, _>(
    EventPriority::NORMAL,
    |event| {
        tracing::info!("{} logged in ({})", event.profile.username, event.player_id);
    },
);
```

### DisconnectEvent

Fired when a player disconnects. The proxy waits for all listeners to finish before cleaning up, so you can do async cleanup here.

| Field | Type | Description |
|-------|------|-------------|
| `player_id` | `PlayerId` | The disconnecting player |
| `username` | `String` | The player's username |
| `last_server` | `Option<ServerId>` | The server they were on, if any |

```rust
ctx.event_bus().subscribe_async::<DisconnectEvent, _>(
    EventPriority::NORMAL,
    |event| Box::pin(async move {
        tracing::info!("{} disconnected", event.username);
    }),
);
```

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

**Type:** Resulted

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
