---
title: Plugin Architecture
description: How Infrarust's four hook layers work — TransportFilter, CodecFilter, EventBus, and LimboHandler — and where each one sits in the connection pipeline.
outline: [2, 3]
---

# Plugin Architecture

Infrarust processes every player connection through a layered pipeline. Each layer operates at a different level of abstraction, from raw TCP bytes to high-level game events. Plugins hook into whichever layer matches what they need to do.

## The pipeline

A connection flows through four layers in order:

```
TCP connection accepted
        │
        ▼
┌───────────────────┐
│  TransportFilter  │  Raw TCP bytes, before Minecraft framing.
│  (Layer 1)        │  Can reject connections at the TCP level.
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

Transport filters operate on raw TCP bytes before the proxy applies Minecraft packet framing. They see every connection, including passthrough-mode connections where the proxy does not decode packets at all.

The trait is defined in `crates/infrarust-api/src/filter/transport.rs`:

```rust
pub trait TransportFilter: Send + Sync {
    fn metadata(&self) -> FilterMetadata;

    fn on_accept<'a>(&'a self, ctx: &'a mut TransportContext)
        -> BoxFuture<'a, FilterVerdict>;

    fn on_client_data<'a>(
        &'a self,
        ctx: &'a mut TransportContext,
        data: &'a mut BytesMut,
    ) -> BoxFuture<'a, FilterVerdict>;

    fn on_server_data<'a>(
        &'a self,
        ctx: &'a mut TransportContext,
        data: &'a mut BytesMut,
    ) -> BoxFuture<'a, FilterVerdict>;

    fn on_close(&self, _ctx: &TransportContext) {}
}
```

`on_accept` fires when a TCP connection arrives. Return `FilterVerdict::Reject` to close it immediately (IP bans, rate limiting). `on_client_data` and `on_server_data` fire on each chunk of bytes flowing in either direction, letting you inspect or modify the raw stream.

`TransportContext` carries connection metadata: remote address, local address, real IP (if behind PROXY protocol), connection time, byte counters, and a type-erased `Extensions` map for sharing state between filters.

```rust
pub enum FilterVerdict {
    Continue,   // No changes, pass data through.
    Modified,   // Data was modified in the BytesMut buffer.
    Reject,     // Drop the connection.
}
```

Transport filters are shared instances (`Send + Sync`). The proxy calls them for every connection. Methods are async because this layer is not on the per-packet hot path.

::: warning
Transport filters are not available to WASM plugins. Direct byte access is restricted to native plugins.
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
            registry.register(Box::new(MyTransportFilter));
        }
        Ok(())
    })
}
```

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
        ctx: &CodecContext,
        packet: &mut RawPacket,
        output: &mut FrameOutput,
    ) -> CodecVerdict;

    fn on_state_change(&mut self, _new_state: ConnectionState) {}
    fn on_compression_change(&mut self, _threshold: i32) {}
    fn on_encryption_enabled(&mut self) {}
    fn on_close(&mut self) {}
}
```

The `filter` method is the hot path. It receives the packet, a context with protocol version and connection state, and a `FrameOutput` for injecting extra packets:

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
    registry.register(Box::new(MyCodecFilterFactory));
}
```

## Layer 3: EventBus

The event bus is the primary hook point for most plugins. It fires typed events at key moments in the player lifecycle. Handlers subscribe with a priority and receive a mutable reference to the event, allowing inspection and modification.

Events fall into categories:

| Category | Events |
|----------|--------|
| Lifecycle | `PreLoginEvent`, `PostLoginEvent`, `DisconnectEvent` |
| Connection | `PlayerChooseInitialServerEvent`, `ServerPreConnectEvent`, `ServerConnectedEvent`, `ServerSwitchEvent`, `KickedFromServerEvent` |
| Chat | `ChatMessageEvent` |
| Proxy | `ProxyPingEvent`, `ProxyInitializeEvent`, `ProxyShutdownEvent`, `ConfigReloadEvent`, `ServerStateChangeEvent` |
| Packet | `RawPacketEvent` |

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
    Allowed,                    // Default — proceed normally.
    Denied { reason: Component },  // Kick the player.
    ForceOffline,               // Skip Mojang authentication.
    ForceOnline,                // Force Mojang authentication.
}
```

`ServerPreConnectEvent` lets you redirect players, send them to limbo, route them to a virtual backend, or deny the connection entirely.

`ChatMessageEvent` lets you allow, deny, or modify messages.

### Priority ordering

Listeners run in priority order from lowest value (FIRST = 0) to highest value (LAST = 255). Each listener sees modifications made by previous listeners.

```rust
EventPriority::FIRST   // 0   — runs first
EventPriority::EARLY   // 64  — before normal
EventPriority::NORMAL  // 128 — default
EventPriority::LATE    // 192 — after normal
EventPriority::LAST    // 255 — runs last
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
}
```

`on_player_enter` determines what happens when the player arrives. The handler returns a `HandlerResult`:

```rust
pub enum HandlerResult {
    Accept,              // Continue to the next handler or the real server.
    Deny(Component),     // Kick the player.
    Hold,                // Keep the player in limbo until complete() is called.
    Redirect(ServerId),  // Send to a specific server.
    SendToLimbo(Vec<String>),  // Chain into another set of limbo handlers.
}
```

When a handler returns `Hold`, the player stays in the void world. The handler uses the `LimboSession` to communicate: send chat messages, display titles, show action bar text. When it's done (player authenticated, server finished booting), it calls `session.complete(result)` to release the player.

Limbo handlers are chained. Each server configuration lists which limbo handlers run and in what order. A player passes through them sequentially.

### Registering a limbo handler

```rust
ctx.register_limbo_handler(Box::new(MyLimboHandler));
```

The handler's `name()` return value must match the name used in server configuration files.

## Filter ordering

Both TransportFilter and CodecFilter use `FilterMetadata` for ordering within their chains:

```rust
pub struct FilterMetadata {
    pub id: &'static str,
    pub priority: FilterPriority,
    pub after: Vec<&'static str>,
    pub before: Vec<&'static str>,
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
| 3 | Codec/transport filters, virtual backends (full packet control) | `CodecFilterFactory`, `TransportFilter`, `VirtualBackendHandler` |

Most plugins only need Tier 1. The auth plugin uses Tier 1 + Tier 2 (events for login flow, limbo for the login screen). Packet-rewriting plugins use Tier 3.

### Virtual backends (Tier 3)

Virtual backends are the most advanced hook point. A `VirtualBackendHandler` takes full control of the client connection and speaks raw Minecraft packets directly. Unlike limbo handlers where the proxy manages the protocol, virtual backends must handle everything themselves: JoinGame, chunks, KeepAlive responses.

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

Virtual backends are routed to via `ServerPreConnectResult::VirtualBackend` in an event handler.

## Choosing the right layer

| You want to... | Use |
|----------------|-----|
| Block IPs, rate-limit connections | TransportFilter |
| Inspect or rewrite raw TCP bytes | TransportFilter |
| Modify, drop, or inject Minecraft packets | CodecFilter |
| React to player login, disconnect, chat | EventBus |
| Redirect players between servers | EventBus (`ServerPreConnectEvent`) |
| Customize the server list ping | EventBus (`ProxyPingEvent`) |
| Hold a player in a waiting room | LimboHandler |
| Build a proxy-hosted minigame | VirtualBackendHandler |
