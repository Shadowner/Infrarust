---
title: Virtual Backend (Planned)
description: A planned capability that lets a WASM plugin act as a custom backend speaking raw Minecraft protocol directly. Not yet implemented.
outline: [2, 3]
---

# Virtual Backend (Planned)

::: warning Not implemented
A Virtual Backend lets a plugin act as the player's server, but the WASM side does not exist yet. The `virtual-backend` capability is defined in the enum, it is not enforced, there is no dispatch wiring in the proxy, and there is no WASM bridge. Do not rely on this page for anything you ship today. For held-player use cases that work now, use [Limbo](./limbo).
:::

## What a Virtual Backend is

A Virtual Backend is a plugin-hosted "server" that speaks raw Minecraft packets directly to the client, with no real backend behind it. The plugin takes full control of the connection and handles the protocol itself: join-game packets, chunks, keep-alive responses, chat, and movement.

The use cases are custom lobbies, mini-games, queue screens with interactivity, and protocol bridges that translate to a non-Minecraft service. A real backend server is not required.

::: info Limbo vs. Virtual Backend
[Limbo](./limbo) holds a player in a minimal idle world that the proxy renders for you. A Virtual Backend hands you the raw packet stream so you render the world yourself. Limbo is available now; Virtual Backend is planned.
:::

## What exists today

The native trait surface lives in `infrarust-api` under `src/virtual_backend/`. These traits are defined but not wired into the proxy, and no WASM plugin can implement them.

### The handler trait

A native Virtual Backend implements `VirtualBackendHandler` (`crates/infrarust-api/src/virtual_backend/handler.rs`):

```rust
pub trait VirtualBackendHandler: Send + Sync {
    fn name(&self) -> &str;

    /// Called when a player session starts on this virtual backend.
    fn on_session_start(&self, session: &dyn VirtualBackendSession) -> BoxFuture<'_, ()>;

    /// Called when a packet is received from the client.
    fn on_packet_received(
        &self,
        session: &dyn VirtualBackendSession,
        packet: &RawPacket,
    ) -> BoxFuture<'_, ()>;

    /// Called when the player session ends (disconnect or server switch).
    fn on_session_end(&self, player_id: PlayerId) -> BoxFuture<'_, ()>;
}
```

The trait is async (`BoxFuture`), which is the native plugin shape, not the synchronous guest `Plugin` trait used by WASM plugins. The doc comments state two hard requirements: `on_session_start` must send a `JoinGame` packet and initial world data or the client disconnects, and `on_packet_received` must answer `KeepAlive` packets.

### The session handle

Inside the callbacks the handler receives a `VirtualBackendSession` (`crates/infrarust-api/src/virtual_backend/session.rs`). The trait is sealed, so only the proxy implements it:

```rust
pub trait VirtualBackendSession: Send + Sync + private::Sealed {
    fn player_id(&self) -> PlayerId;
    fn profile(&self) -> &GameProfile;
    fn protocol_version(&self) -> ProtocolVersion;

    /// Sends a raw packet to the client.
    fn send_packet(&self, packet: &RawPacket) -> Result<(), PlayerError>;

    /// Sends a chat message to the player (convenience wrapper).
    fn send_message(&self, message: Component) -> Result<(), PlayerError>;

    /// Switches the player to a real backend server.
    fn switch_server(&self, target: ServerId) -> BoxFuture<'_, Result<(), PlayerError>>;

    /// Disconnects the player with a reason message.
    fn disconnect(&self, reason: Component) -> BoxFuture<'_, ()>;
}
```

| Method | Returns | Purpose |
|--------|---------|---------|
| `player_id()` | `PlayerId` | Identifier for the connected player. |
| `profile()` | `&GameProfile` | The player's game profile (name, UUID, properties). |
| `protocol_version()` | `ProtocolVersion` | Negotiated client protocol version. |
| `send_packet(packet)` | `Result<(), PlayerError>` | Sends a raw packet to the client. |
| `send_message(message)` | `Result<(), PlayerError>` | Sends a chat component to the player. |
| `switch_server(target)` | `BoxFuture<Result<(), PlayerError>>` | Moves the player to a real backend. |
| `disconnect(reason)` | `BoxFuture<()>` | Disconnects the player with a reason. |

`send_packet` and `send_message` return `Err(PlayerError::SendFailed)` when delivery fails.

### The capability

`Capability::VirtualBackend` is a variant in the capability enum (`crates/infrarust-api/src/permissions.rs`), and its kebab-case config string is `virtual-backend`. Its presence in the enum is the full extent of the integration today.

```rust
// crates/infrarust-api/src/permissions.rs
Capability::VirtualBackend => "virtual-backend",
```

::: danger Capability not enforced
`virtual-backend` is not in the [baseline](./capabilities). Listing it in `permissions` does insert `Capability::VirtualBackend` into the granted set (unlike `transport-filter`, it is not rejected by `from_config_strings`), but no host code reads it, so granting it has no effect today.
:::

## What does not exist

Three pieces are missing before a WASM plugin can host a backend:

- Enforcement: nothing in the host checks `virtual-backend` (the linker gates `event-bus`, `player-read`, `command`, `scheduler`, `config-read`, `server-manage`, `ban`, `codec-filter`, `limbo`, and `raw-packet`, but never this one).
- Dispatch: the proxy has no path that routes a player's connection to a registered `VirtualBackendHandler`.
- WASM bridge: there is no host wrapper that forwards `on_session_start`, `on_packet_received`, and `on_session_end` to guest exports.

The WIT contract (`infrarust:plugin@0.2.3`) reflects this. Its header comment states the scope of v0.2 and lists Virtual Backend as out of scope:

```wit
// v0.2 adds raw-packet events, codec filters, limbo, and
// custom permission checkers; virtual backend stays deferred to a later minor.
```

## The planned approach

The loader spec describes how Virtual Backend will reach WASM once the native dispatch is operational. It reuses the marker-plus-proxy pattern already used for Limbo and custom permission checkers.

```mermaid
sequenceDiagram
    participant Client
    participant Proxy
    participant Host as WASM host wrapper
    participant Guest as WASM plugin

    Client->>Proxy: connect
    Proxy->>Guest: pre-connect event
    Guest-->>Proxy: outcome = virtual-backend(handler-id)
    Proxy->>Host: wrap marker in WasmVirtualBackendHandler
    Host->>Guest: on_session_start(session)
    Client->>Host: raw packet
    Host->>Guest: on_packet_received(session, packet)
    Client->>Host: disconnect
    Host->>Guest: on_session_end(player_id)
```

The flow, per the spec:

1. A guest plugin registers a `handler-id`.
2. On a pre-connect event whose result carries a backend handler, the guest returns a `virtual-backend(handler-id)` marker in its `event-outcome`.
3. The host wraps that marker in a native `Box<dyn VirtualBackendHandler>` (a `WasmVirtualBackendHandler`) that dispatches each callback to the matching guest export.

Shipping this is a minor WIT bump, not a refactor, because the bridge shape is already designed. The spec ties the timeline to the native side: Virtual Backend is "Tier 3 défini, pas pleinement opérationnel" (defined, not fully operational), so the WASM exposure waits on that maturing first.

## Until then

Use [Limbo](./limbo) to hold a player without a backend. The Limbo path is exposed to WASM through `infrarust:plugin@0.2.3` (`register-limbo-handler`, `hold-with-timeout`, `on-session-end`), covers idle worlds and queue screens, and runs today.

If you need packet-level control before Virtual Backend lands, the [`raw-packet` capability and `RawPacketEvent`](./capabilities) are the relevant contract surface to track; note that `raw-packet` is defined in the WIT contract and not yet exposed by the SDK.

## See also

- [Limbo](./limbo): the held-player path that works now.
- [Capabilities](./capabilities): baseline grants, opt-in capabilities, and config strings.
- [Architecture](./architecture): how the host loads and dispatches to WASM plugins.
- [Native plugin development](../dev/getting-started): where the `VirtualBackendHandler` and `VirtualBackendSession` traits live.
- [Configuration](../../configuration/): the `permissions` list format.
