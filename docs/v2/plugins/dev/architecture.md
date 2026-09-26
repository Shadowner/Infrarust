---
title: Plugin Architecture
description: How Infrarust's four native hook layers work (TransportFilter, CodecFilter, EventBus, and LimboHandler) and where each one sits in the connection pipeline.
outline: [2, 3]
---

# Plugin Architecture

Infrarust processes every player connection through a layered pipeline. Each layer operates at a different level of abstraction, from accepted TCP connections to high-level game events. Plugins hook into whichever layer matches what they need to do.

Native plugins are Rust crates compiled into the proxy binary. They run in-process with full access to every proxy service, including the transport layer that gates TCP connections. There is no sandbox: a native plugin is trusted code that the operator chose to compile in, and a misbehaving one can crash or stall the proxy. WASM plugins use a separate capability-gated model and do not reach the lowest layers described here. This page covers the native layers.

## The pipeline

A connection flows through four layers in order:

```
TCP connection accepted
        │
        ▼
┌───────────────────┐
│  TransportFilter  │  Accepted TCP connections, before framing.
│  (Layer 1)        │  Can reject them; told when they close.
└───────┬───────────┘
        │
        ▼
┌───────────────────┐
│   CodecFilter     │  Framed Minecraft packets (RawPacket).
│   (Layer 2)       │  Synchronous, per-connection instances.
└───────┬───────────┘
        │
        ▼
┌───────────────────┐
│    EventBus       │  High-level events (login, chat, kicks).
│    (Layer 3)      │  Async handlers, priority-ordered.
└───────┬───────────┘
        │
        ▼
┌───────────────────┐
│  LimboHandler     │  Session-level control. Holds players
│  (Layer 4)        │  in proxy-hosted worlds.
└───────────────────┘
```

Each layer is independent. A plugin can hook into one layer, multiple layers, or all four depending on its needs. A rate-limiter plugin might only use TransportFilter. An auth plugin uses EventBus for login events and LimboHandler to hold unauthed players. A packet inspector uses CodecFilter.

## Layer 1: TransportFilter

A transport filter is a connection gate. The proxy asks it about every accepted TCP connection before it reads a single Minecraft packet, and tells it when that connection closes. Transport filters see every connection: intercepted ones (`offline`, `client_only`), forwarded ones (`passthrough`, `zero_copy`, `server_only`) where the proxy never decodes the game traffic, and pre-1.7 legacy clients.

The trait is defined in `crates/infrarust-api/src/filter/transport.rs`:

```rust
pub trait TransportFilter: Send + Sync {
    fn metadata(&self) -> FilterMetadata;

    fn on_accept<'a>(&'a self, ctx: &'a mut TransportContext)
        -> BoxFuture<'a, FilterVerdict>;

    fn on_close(&self, _ctx: &TransportContext) {}
}

pub enum FilterVerdict {
    Continue,   // Let the connection through to the next filter.
    Reject,     // Close it without an answer.
}
```

Transport filters are shared instances (`Send + Sync`): one filter object serves every connection. `on_accept` is async because this layer is not on the per-packet hot path.

### on_accept

`on_accept` runs in the connection's own task, never in the accept loop. A filter that takes its time, such as an anti-VPN lookup or a database query, delays only the connection it is looking at; the proxy keeps accepting and serving everyone else. It runs after the PROXY protocol header has been decoded (when [`receive_proxy_protocol`](../../reference/proxy-protocol) is on) and before the handshake is read.

The filters run one after another, in [chain order](#filter-ordering). The first `Reject` stops the chain. The proxy then closes the socket without sending anything, not even a kick message or a server list answer, and fires a [`ConnectionRejectedEvent`](./events#connectionrejectedevent) with the reason `RejectReason::Plugin { plugin_id }`, where `plugin_id` is the plugin that registered the filter (`None` for a filter the proxy registered itself) and `virtual_host` is `None` because no handshake was read. To refuse a player with a message, deny the `ConnectionHandshakeEvent` instead.

Each call has a time limit, [`[events] transport_filter_timeout`](../../configuration/global#plugin-event-handlers) (5 seconds by default). A filter that panics, or that has not answered when the limit runs out, rejects the connection: transport filters fail closed, like the ban provider does for logins. The proxy drops the unfinished future, logs a warning that names the filter and its plugin, and fires the same `ConnectionRejectedEvent`. The fault only costs that one connection: the next one runs the filter again as usual. If the proxy shuts down while a filter is running, the call is cancelled and the connection closed.

### on_close

`on_close` is called exactly once for every connection the filter let through with `Continue`, when that connection ends, whatever ends it: a server list ping or a play session that finished, a rejection later in the pipeline (unknown domain, IP filter, ban, a denied `ConnectionHandshakeEvent`), an I/O error, a panic in the proxy's connection code, or a proxy shutdown. This holds in every proxy mode and for legacy clients.

`on_accept` and `on_close` pair up per filter:

- When a filter rejects a connection, or panics or times out on it, the filters before it in the chain that had already returned `Continue` get their `on_close` at once. The filter that rejected gets none, and neither do the filters after it, which never saw the connection.
- Filters are closed in the reverse of chain order.
- `on_close` runs once the client socket is closed. It is synchronous, so keep it short. A panic in it is caught and logged, and the other filters still get theirs.
- A filter whose plugin is disabled while one of its connections is open still gets `on_close` for that connection, so its bookkeeping stays balanced. Connections accepted after the disable no longer run it.

This makes counters safe: increment in `on_accept` when you return `Continue`, decrement in `on_close`.

### TransportContext

| Field | Type | Description |
|-------|------|-------------|
| `remote_addr` | `SocketAddr` | The TCP peer. Behind a load balancer this is the load balancer |
| `local_addr` | `SocketAddr` | The listener address the connection arrived on |
| `real_ip` | `Option<IpAddr>` | The client IP from the PROXY protocol header when `receive_proxy_protocol` is on, `None` otherwise. An IPv4-mapped IPv6 address is given as IPv4 |
| `connection_time` | `Instant` | When the connection was accepted |
| `connection_id` | `u64` | Non-zero and unique for the life of the proxy process. If the connection logs in, it is also the player's `PlayerId` (`player.id().as_u64()`) and the `connection_id` of its codec filters' `CodecSessionInit` |
| `extensions` | `Extensions` | A type map shared by the transport filters of this connection |

`on_close` receives the same context `on_accept` filled, extensions included, so a filter can keep per-connection state there (a start time, a lookup result) and read it back when the connection ends. The extensions stay with the transport filters: they are not copied into the proxy's connection pipeline, because no plugin-facing API reads the pipeline's internal state and nothing downstream could look at them.

### What transport filters cannot do

Transport filters have no access to the bytes of a connection. Feeding them the stream would mean wrapping every socket and copying every byte through the filter chain, which would disable the `zero_copy` mode's `splice` forwarding and slow every other mode down, so it is not supported. To look at or change game traffic, use a [codec filter](#layer-2-codecfilter) (intercepted modes) or an [event](#layer-3-eventbus).

A transport filter cannot answer the client either: a rejected connection is closed silently.

Here is a filter that caps the number of open connections per client IP:

```rust
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;

use infrarust_api::prelude::*;

struct PerIpLimit {
    max: usize,
    open: Mutex<HashMap<IpAddr, usize>>,
}

fn client_ip(ctx: &TransportContext) -> IpAddr {
    ctx.real_ip.unwrap_or(ctx.remote_addr.ip())
}

impl TransportFilter for PerIpLimit {
    fn metadata(&self) -> FilterMetadata {
        FilterMetadata::new("per_ip_limit")
    }

    fn on_accept<'a>(&'a self, ctx: &'a mut TransportContext) -> BoxFuture<'a, FilterVerdict> {
        let mut open = self.open.lock().unwrap();
        let count = open.entry(client_ip(ctx)).or_insert(0);
        let verdict = if *count < self.max {
            *count += 1;
            FilterVerdict::Continue
        } else {
            FilterVerdict::Reject
        };
        Box::pin(async move { verdict })
    }

    fn on_close(&self, ctx: &TransportContext) {
        // Only connections that got Continue reach on_close,
        // so this always undoes an increment.
        let ip = client_ip(ctx);
        let mut open = self.open.lock().unwrap();
        if let Some(count) = open.get_mut(&ip) {
            *count -= 1;
            if *count == 0 {
                open.remove(&ip);
            }
        }
    }
}
```

::: warning
Transport filters are not available to WASM plugins. The `transport-filter` capability can only be held by trusted native plugins.
:::

### Registering a transport filter

Register transport filters through `PluginContext::transport_filters()` during `on_enable`:

```rust
fn on_enable<'a>(
    &'a self,
    ctx: &'a dyn PluginContext,
) -> BoxFuture<'a, Result<(), PluginError>> {
    Box::pin(async move {
        if let Some(registry) = ctx.transport_filters() {
            registry
                .register(Box::new(MyTransportFilter))
                .map_err(|e| PluginError::InitFailed(e.to_string()))?;
        }
        Ok(())
    })
}
```

The filter id is owned by the plugin that registers it. Registering an id that another plugin already owns fails with `FilterRegistryError::OwnedBy`, and `unregister` only removes your own filters. When the plugin is disabled, the proxy removes its transport filters and rebuilds the chain, so the next accepted connection no longer runs them.

## Layer 2: CodecFilter

Codec filters operate on framed Minecraft packets (`RawPacket`). They run inline in the proxy's packet-forwarding loop for every single packet, so they must be fast (under 1 microsecond per call).

This layer uses a factory pattern. You register a `CodecFilterFactory` once globally, and the proxy creates per-connection `CodecFilterInstance` objects from it. Each instance holds mutable state for one connection and one side (client-side or server-side).

The factory trait (`crates/infrarust-api/src/filter/codec.rs`):

```rust
pub trait CodecFilterFactory: Send + Sync {
    fn metadata(&self) -> FilterMetadata;
    fn create(&self, ctx: &CodecSessionInit) -> Box<dyn CodecFilterInstance>;
}
```

The per-connection instance trait:

```rust
pub trait CodecFilterInstance: Send {
    fn filter(
        &mut self,
        packet: &mut RawPacket,
        output: &mut FrameOutput,
    ) -> CodecVerdict;

    fn on_state_change(&mut self, _new_state: ConnectionState) {}
    fn on_compression_change(&mut self, _threshold: i32) {}
    fn on_encryption_enabled(&mut self) {}
    fn on_close(&mut self) {}
}
```

The `filter` method is the hot path. It receives the packet and a `FrameOutput` for injecting extra packets. The instance learns about protocol state through the `on_state_change`, `on_compression_change`, and `on_encryption_enabled` callbacks rather than a per-call context, and the protocol version and side were fixed at creation time by `CodecSessionInit`:

```rust
pub enum CodecVerdict {
    Pass,     // Let the packet through (possibly modified in place).
    Drop,     // Discard the packet.
    Replace,  // Replace with packets injected via FrameOutput.
    Error(CodecFilterError),
}
```

`FrameOutput` lets you inject packets before or after the current one:

```rust
output.inject_before(RawPacket::new(0x01, data));
output.inject_after(RawPacket::new(0x02, data));
```

The factory's `create` method receives a `CodecSessionInit` struct with the client's protocol version, connection ID, remote address, and which `ConnectionSide` (client or server) this instance will handle. The proxy calls `create` twice per session: once for the client-side, once for the server-side.

`CodecFilterInstance` is `Send` but not `Sync`. Each instance lives in a single tokio task. All methods are synchronous (no async) because they run on the packet hot path.

### Registering a codec filter

```rust
if let Some(registry) = ctx.codec_filters() {
    if let Err(e) = registry.register(Box::new(MyCodecFilterFactory)) {
        // FilterRegistryError::OwnedBy: another plugin or the proxy owns this id.
        tracing::warn!("codec filter not registered: {e}");
    }
}
```

The same ownership rules apply as for transport filters. The proxy builds the codec chain of each session when it starts, so disabling the plugin removes its codec filters from every connection opened afterwards; sessions already running keep the instances they were created with until they close.

## Layer 3: EventBus

The event bus is the primary hook point for most plugins. It fires typed events at key moments in the player lifecycle. Handlers subscribe with a priority and receive a mutable reference to the event, allowing inspection and modification.

Events fall into categories:

| Category | Events |
|----------|--------|
| Handshake | `ConnectionHandshakeEvent`, `ConnectionRejectedEvent` |
| Lifecycle | `PreLoginEvent`, `OnlineAuthFailed`, `GameProfileRequestEvent`, `PermissionsSetupEvent`, `LoginEvent`, `PostLoginEvent`, `DisconnectEvent` |
| Connection | `PlayerChooseInitialServerEvent`, `ServerPreConnectEvent`, `ServerConnectedEvent`, `ServerPostConnectEvent`, `KickedFromServerEvent`, `LimboEnterEvent`, `LimboExitEvent` |
| Chat | `ChatMessageEvent`, `CommandExecuteEvent` |
| Proxy | `ProxyPingEvent`, `ProxyInitializeEvent`, `ProxyShutdownEvent`, `ConfigReloadEvent`, `ServerStateChangeEvent`, `BackendHealthEvent` |
| Packet (Tier 3) | `RawPacketEvent` (packet subscriptions only) |
| Plugin-defined | `NamedEvent` and your own event types, fired with `fire` ([custom events](./events#custom-events)) |

The [events page](./events) documents each event's fields and result type.

### Subscribing to events

```rust
ctx.event_bus().subscribe::<PostLoginEvent, _>(
    EventPriority::NORMAL,
    |event| {
        tracing::info!("Player {} joined", event.profile.username);
    },
);
```

Async handlers work the same way:

```rust
ctx.event_bus().subscribe_async::<PreLoginEvent, _>(
    EventPriority::EARLY,
    |event| {
        Box::pin(async move {
            // async work here
        })
    },
);
```

### Resulted events

Some events implement `ResultedEvent`, which means handlers can change the outcome. The proxy reads the final result after all handlers have run.

For example, `PreLoginEvent` supports these results:

```rust
pub enum PreLoginResult {
    Allowed,                       // Default. Proceed normally.
    Denied { reason: Component },  // Kick the player.
    ForceOffline,                  // Skip Mojang authentication.
    ForceOnline,                   // Force Mojang authentication.
}
```

`ServerPreConnectEvent` lets you redirect players to another backend, send them to a limbo handler chain, or deny the connection entirely. Routing to a virtual backend is not wired into the proxy yet (see below).

`ChatMessageEvent` lets you allow, deny, or modify messages, signed ones included. `CommandExecuteEvent` does the same for commands, before the proxy runs its own.

### Priority ordering

Listeners run in priority order from lowest value (FIRST = 0) to highest value (LAST = 255). Each listener sees modifications made by previous listeners.

```rust
EventPriority::FIRST   // 0   runs first
EventPriority::EARLY   // 64  before normal
EventPriority::NORMAL  // 128 default
EventPriority::LATE    // 192 after normal
EventPriority::LAST    // 255 runs last
```

Use `EventPriority::custom(u8)` for values between the named constants.

### Packet-level events

For plugins that need to see individual packets without writing a CodecFilter, the event bus supports packet subscriptions filtered by packet ID, connection state, and direction:

```rust
ctx.event_bus().subscribe_packet_typed(
    PacketFilter {
        packet_id: 0x03,
        state: ConnectionState::Play,
        direction: PacketDirection::Serverbound,
    },
    EventPriority::NORMAL,
    |event: &mut RawPacketEvent| {
        event.drop_packet(); // Silently discard
    },
);
```

The proxy skips event dispatch for packets that have no listeners registered, so unused packet subscriptions have zero overhead.

`RawPacketEvent` is not an `Event`, so it cannot be passed to `subscribe` or `subscribe_async`. Packet subscriptions are the only way to receive it.

## Layer 4: LimboHandler

Limbo handlers give a plugin full control over a player's session without requiring raw protocol knowledge. The proxy hosts the player in a void world and manages the Minecraft protocol (JoinGame, KeepAlive, chunks). The handler receives high-level callbacks for chat, commands, and player entry.

The trait is defined in `crates/infrarust-api/src/limbo/handler.rs`:

```rust
pub trait LimboHandler: Send + Sync {
    fn name(&self) -> &str;

    fn on_player_enter<'a>(
        &'a self,
        session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult>;

    fn on_command<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
        _command: &'a str,
        _args: &'a [&'a str],
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn on_chat<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
        _message: &'a str,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn on_disconnect(&self, _player_id: PlayerId) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn on_session_end(&self, _player_id: PlayerId, _reason: SessionEndReason)
        -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}
```

`on_disconnect` fires only when the client drops. `on_session_end` fires on every terminal outcome (released, kicked, redirected, timed out, or shutdown, via the `SessionEndReason` enum), so it is the right place to tear down retained state uniformly. The per-session cancellation token from `LimboSession::cancellation_token()` is cancelled around the same time.

`on_player_enter` determines what happens when the player arrives. The handler returns a `HandlerResult`:

```rust
pub enum HandlerResult {
    Accept,              // Continue to the next handler or the real server.
    Deny(Component),     // Kick the player.
    Hold,                // Keep the player in limbo until complete() is called.
    Redirect(ServerId),  // Send to a specific server.
    SendToLimbo(Vec<String>),  // Chain into another set of limbo handlers.
    HoldWithTimeout {    // Like Hold, but auto-complete after the deadline.
        after: Duration,
        on_timeout: Box<HandlerResult>,
    },
}
```

When a handler returns `Hold`, the player stays in the void world. The handler uses the `LimboSession` to communicate: send chat messages, display titles, show action bar text. When it's done (player authenticated, server finished booting), it calls `session.complete(result)` to release the player. `HoldWithTimeout` is the same, except the engine owns a timer and applies `on_timeout` if `complete()` is not called within `after`. The deadline holds even if the handler's own task dies, and `on_timeout` must be a terminal result (`Accept`, `Deny`, `Redirect`, or `SendToLimbo`); a nested hold there is treated as `Accept`.

Limbo handlers are chained. Each server configuration lists which limbo handlers run and in what order. A player passes through them sequentially.

### Registering a limbo handler

```rust
ctx.register_limbo_handler(Box::new(MyLimboHandler))?;
```

The handler's `name()` return value must match the name used in server configuration files. Names are unique across plugins, registration works at any time, and removing a handler releases the players it holds. See [Limbo handlers](./api#limbo-handlers).

## Filter ordering

Both TransportFilter and CodecFilter use `FilterMetadata` for ordering within their chains:

```rust
pub struct FilterMetadata {
    pub id: String,
    pub priority: FilterPriority,
    pub after: Vec<String>,
    pub before: Vec<String>,
}
```

`FilterPriority` controls the base execution order:

```rust
pub enum FilterPriority {
    First  = 0,  // Security filters.
    Early  = 1,
    Normal = 2,  // Default.
    Late   = 3,
    Last   = 4,  // Logging filters.
}
```

The `after` and `before` fields express explicit dependencies between filters by ID. If filter A lists filter B in its `after` field, A is guaranteed to run after B regardless of priority.

## Plugin tiers

The layers map to three plugin complexity tiers:

| Tier | Capability | Key traits |
|------|-----------|------------|
| 1 | Event listeners, commands, services | `Plugin`, `EventBus` |
| 2 | Limbo handlers (proxy manages protocol) | `LimboHandler`, `LimboSession` |
| 3 | Codec and transport filters, full packet control | `CodecFilterFactory`, `TransportFilter` |

Most plugins only need Tier 1. The auth plugin uses Tier 1 plus Tier 2 (events for login flow, limbo for the login screen). Packet-rewriting plugins use Tier 3.

### Virtual backends (planned)

A virtual backend is a proxy-hosted "server" that speaks raw Minecraft packets directly to the client. Unlike a limbo handler, where the proxy manages the protocol for you, a virtual backend handles everything itself: JoinGame, chunks, KeepAlive responses. The `VirtualBackendHandler` trait exists in `infrarust_api`:

```rust
pub trait VirtualBackendHandler: Send + Sync {
    fn name(&self) -> &str;
    fn on_session_start(&self, session: &dyn VirtualBackendSession)
        -> BoxFuture<'_, ()>;
    fn on_packet_received(&self, session: &dyn VirtualBackendSession,
        packet: &RawPacket) -> BoxFuture<'_, ()>;
    fn on_session_end(&self, player_id: PlayerId) -> BoxFuture<'_, ()>;
}
```

The proxy cannot route a player to a virtual backend yet, and no event result selects one: the `ServerPreConnectResult::VirtualBackend` arm was removed because the proxy never acted on it. Treat virtual backends as planned, not available, and use a limbo handler for proxy-hosted screens today.

::: info Planned
Virtual backend routing is not implemented in the proxy on v2.0.0-beta.3. The trait is published so the API can stabilize ahead of the runtime support.
:::

## Choosing the right layer

| You want to... | Use |
|----------------|-----|
| Block IPs, rate-limit connections | TransportFilter |
| Track open connections (per-IP caps, connection logs) | TransportFilter |
| Refuse a connection with a message | EventBus (`ConnectionHandshakeEvent`) |
| Modify, drop, or inject Minecraft packets | CodecFilter |
| React to player login, disconnect, chat | EventBus |
| Redirect players between servers | EventBus (`ServerPreConnectEvent`) |
| Customize the server list ping | EventBus (`ProxyPingEvent`) |
| Hold a player in a waiting room | LimboHandler |
| Build a proxy-hosted screen or minigame | LimboHandler (VirtualBackendHandler is planned) |
