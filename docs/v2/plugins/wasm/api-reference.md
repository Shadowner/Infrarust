---
title: WIT API Reference
description: "The infrarust:plugin@0.2.3 WIT contract: package, world, types, and guest exports."
outline: [2, 3]
---

# WIT API Reference

This page documents the low-level WIT contract that defines the host/guest boundary for Infrarust WASM plugins. It lists every interface, type, and signature in the contract.

You rarely call these signatures directly. The SDK generates bindings from this contract and presents Rust wrappers on top of them, so the author-facing API lives on the per-capability pages: [Events](./events), [Services](./services), [Limbo](./limbo), and [Codec Filters](./codec-filters). Read this page when you need the exact wire shape behind a wrapper.

::: info
The contract is `infrarust:plugin@0.2.3`. The `WORLD_VERSION` constant in `infrarust-plugin-wit/src/lib.rs` still reads `0.2.0` and is stale; the authoritative version is the package declaration in `wit/world.wit`.
:::

## Package and world

The contract is one package split across files under `crates/infrarust-plugin-wit/wit/`. The `plugin` world lists 11 imports (host capabilities) and 2 exports (the guest surface).

```wit
package infrarust:plugin@0.2.3;

world plugin {
    import types;
    import log;
    import event-bus;
    import player-registry;
    import server-manager;
    import ban-service;
    import config-service;
    import command-manager;
    import scheduler;
    import limbo;
    import codec-registry;

    export guest;
    export codec-filter;
}
```

### Imports

| Import | Capability gate | Purpose |
|--------|-----------------|---------|
| `types` | none | Shared aliases, enums, records, and variants. |
| `log` | none | `trace`/`debug`/`info`/`warn`/`error` to the host log. |
| `event-bus` | `event-bus` | Subscribe and unsubscribe to event kinds by priority. |
| `player-registry` | `player-read` (some methods `player-write` / `raw-packet`) | Look up players and act on the `player` resource. |
| `server-manager` | `server-manage` | Read server state, start and stop backends. |
| `ban-service` | `ban` | Create, remove, and query bans. |
| `config-service` | `config-read` | Read server configs and config values. |
| `command-manager` | `command` | Register and unregister proxy commands. |
| `scheduler` | `scheduler` | Schedule one-shot and interval callbacks. |
| `limbo` | `limbo` | Register limbo handlers and act on the session resources. |
| `codec-registry` | `codec-filter` | Register and unregister codec filters. |

### Exports

| Export | Purpose |
|--------|---------|
| `guest` | Lifecycle, the unified event dispatch, and every marker+proxy callback entry point. |
| `codec-filter` | The per-session `filter-instance` resource and its `create` factory. |

::: tip Baseline vs opt-in capabilities
Every WASM plugin gets the baseline capabilities (`event-bus`, `player-read`, `player-write`, `command`, `scheduler`, `config-read`). The rest (`ban`, `server-manage`, `codec-filter`, `limbo`, `raw-packet`) are opt-in: list them in your plugin's TOML permissions. See [Capabilities](./capabilities) for the full model.
:::

## Core types

All shared types live in `types.wit` and are used across every other interface.

### Primitive aliases

| Alias | Underlying type | Notes |
|-------|-----------------|-------|
| `player-id` | `u64` | Identifies a connected player. |
| `server-id` | `string` | Backend server config id. |
| `protocol-version` | `s32` | Minecraft protocol version number. |
| `task-handle` | `u64` | Returned by `scheduler.delay` / `scheduler.interval`. |
| `listener-handle` | `u64` | Returned by `event-bus.subscribe`. |
| `handler-id` | `u64` | A guest-owned id behind a registered handler (limbo handler, permission checker, or codec factory). The host wraps it in a native marker+proxy. |
| `uuid` | `string` | Player UUID as a string. |
| `event-priority` | `u8` | 1:1 with native `EventPriority(u8)`. The SDK exposes the named levels (first=0, early=64, normal=128, late=192, last=255). |
| `component` | `string` | A pre-serialized Minecraft text-component JSON string (see below). |

::: info `component` is JSON, not a record
WIT has no recursive types, so the recursive native `Component` cannot be a WIT record. Rich text crosses the boundary as a JSON string already serialized to the Minecraft text-component format. The SDK's `Component` builder produces this string for you.
:::

### Enums

| Enum | Variants |
|------|----------|
| `connection-state` | `handshake`, `status`, `login`, `configuration`, `play` |
| `packet-direction` | `serverbound`, `clientbound` |
| `server-state` | `online`, `offline`, `starting`, `stopping`, `sleeping`, `crashed` |
| `proxy-mode` | `passthrough`, `zero-copy`, `client-only`, `offline`, `server-only` |
| `permission-level` | `player`, `admin` |
| `connection-side` | `client-side`, `server-side` (defined in `codec-filter`) |

### Records

```wit
record profile-property {
    name: string,
    value: string,
    signature: option<string>,
}
record game-profile {
    uuid: uuid,
    username: string,
    properties: list<profile-property>,
}
record server-address {
    host: string,
    port: u16,
}
record server-config {
    id: server-id,
    network: option<string>,
    addresses: list<server-address>,
    domains: list<string>,
    proxy-mode: proxy-mode,
    limbo-handlers: list<string>,
    max-players: u32,
    disconnect-message: option<string>,
    send-proxy-protocol: bool,
    has-server-manager: bool,
}
record title-data {
    title: component,
    subtitle: component,
    fade-in-ticks: s32,
    stay-ticks: s32,
    fade-out-ticks: s32,
}
record raw-packet {
    packet-id: s32,
    data: list<u8>,
}
record ban-entry {
    target: ban-target,
    reason: option<string>,
    expires-at: option<u64>,
    created-at: u64,
    source: string,
}
record plugin-dependency {
    id: string,
    optional: bool,
}
record plugin-metadata {
    id: string,
    name: string,
    version: string,
    authors: list<string>,
    description: option<string>,
    dependencies: list<plugin-dependency>,
}
```

`raw-packet` is kept for `player.send-packet` (gated by the `raw-packet` capability). The `raw-packet` *event* is contract-only; see [Events](./events).

### Variants

```wit
variant ban-target {
    ip(string),
    username(string),
    uuid(uuid),
}
variant player-error {
    not-active,
    disconnected,
    send-failed(string),
    server-not-found(string),
    switch-failed(string),
}
variant service-error {
    not-found(string),
    operation-failed(string),
    unavailable(string),
}
```

The codec variants (`codec-filter-error`, `filter-output`) live in `codec-filter`; see [The codec-filter export](#the-codec-filter-export).

## The guest export

`guest.wit` defines the lifecycle functions, the unified event dispatch, and the marker+proxy callback entry points. Per-event records and result variants are declared here and feed two umbrella variants: `event` (host to guest) and `event-outcome` (guest to host).

### Lifecycle

```wit
metadata: func() -> plugin-metadata;
on-enable: func() -> result<_, string>;
on-disable: func() -> result<_, string>;
```

::: tip The SDK trait is synchronous
The generated guest entry points map to the SDK's `Plugin` trait, which is synchronous:

```rust
pub trait Plugin: 'static {
    fn metadata(&self) -> PluginMetadata;
    fn on_enable(&self, ctx: &Context) -> Result<(), String>;
    fn on_disable(&self, _ctx: &Context) -> Result<(), String> { Ok(()) }
}
```

This differs from the native `BoxFuture`-based trait. See [native getting-started](../dev/getting-started) for the native API.
:::

### Event dispatch

The host calls `handle-event` once per subscribed kind, passing the `listener-handle` returned by `event-bus.subscribe` and the `event` payload. The guest returns an `event-outcome`; `none` keeps native behavior.

```wit
handle-event: func(listener: listener-handle, ev: event) -> event-outcome;
```

The `event` variant carries 17 arms. Each maps to a `record` (or a unit arm for the data-less kinds):

```wit
variant event {
    pre-login(pre-login-event),
    post-login(post-login-event),
    disconnect(disconnect-event),
    online-auth-failed(online-auth-failed-event),
    permissions-setup(permissions-setup-event),
    server-pre-connect(server-pre-connect-event),
    server-connected(server-connected-event),
    server-switch(server-switch-event),
    kicked-from-server(kicked-from-server-event),
    player-choose-initial-server(player-choose-initial-server-event),
    proxy-ping(proxy-ping-event),
    proxy-initialize,
    proxy-shutdown,
    config-reload,
    server-state-change(server-state-change-event),
    chat-message(chat-message-event),
    raw-packet(raw-packet-event),
}
```

Eight of those kinds can change the outcome (`none` keeps native behavior):

```wit
variant event-outcome {
    none,
    pre-login(pre-login-result),
    server-pre-connect(server-pre-connect-result),
    kicked-from-server(kicked-from-server-result),
    player-choose-initial-server(player-choose-initial-server-result),
    chat-message(chat-message-result),
    proxy-ping(ping-response),
    permissions-setup(permissions-setup-result),
    raw-packet(raw-packet-result),
}
```

::: warning Not every contract kind is exposed by the SDK
`raw-packet` is defined in the WIT contract but not yet exposed by the SDK. The SDK exposes the observe-only kinds (`post-login`, `disconnect`, `online-auth-failed`, `permissions-setup`, `server-connected`, `server-switch`, `server-state-change`, plus the data-less `proxy-initialize`, `proxy-shutdown`, `config-reload`) and the modifiable kinds (`pre-login`, `server-pre-connect`, `kicked-from-server`, `player-choose-initial-server`, `proxy-ping`, `chat-message`). `permissions-setup` is an observe record whose `permissions-setup-result::custom(handler-id)` outcome registers a guest permission checker. See [Events](./events) for the typed surface.
:::

### Event records and results

The full set of per-event records and their result variants:

```wit
record pre-login-event {
    profile: game-profile,
    remote-addr: string,
    protocol-version: protocol-version,
    server-domain: string,
}
variant pre-login-result { allowed, denied(component), force-offline, force-online }

record post-login-event {
    profile: game-profile,
    player-id: player-id,
    protocol-version: protocol-version,
}

record disconnect-event {
    player-id: player-id,
    username: string,
    last-server: option<server-id>,
}

record online-auth-failed-event { username: string }

record permissions-setup-event {
    player-id: player-id,
    profile: game-profile,
    online-mode: bool,
}
variant permissions-setup-result { use-default, custom(handler-id) }

record server-pre-connect-event {
    player-id: player-id,
    profile: game-profile,
    original-server: server-id,
}
variant server-pre-connect-result {
    allowed,
    connect-to(server-id),
    send-to-limbo(list<string>),
    denied(component),
}

record server-connected-event { player-id: player-id, server: server-id }

record server-switch-event {
    player-id: player-id,
    previous-server: server-id,
    new-server: server-id,
}

record kicked-from-server-event {
    player-id: player-id,
    server: server-id,
    reason: component,
}
variant kicked-from-server-result {
    disconnect-player(component),
    redirect-to(server-id),
    send-to-limbo(list<string>),
    notify(component),
}

record player-choose-initial-server-event {
    player-id: player-id,
    profile: game-profile,
    initial-server: server-id,
}
variant player-choose-initial-server-result {
    allowed,
    redirect(server-id),
    send-to-limbo(list<string>),
}

record ping-response {
    description: component,
    max-players: s32,
    online-players: s32,
    protocol-version: protocol-version,
    version-name: string,
    favicon: option<string>,
}
record proxy-ping-event { remote-addr: string, response: ping-response }

record server-state-change-event {
    server: server-id,
    old-state: server-state,
    new-state: server-state,
}

record chat-message-event { player-id: player-id, message: string }
variant chat-message-result { allow, deny(component), modify(string) }

record raw-packet-event {
    player-id: player-id,
    direction: packet-direction,
    packet: raw-packet,
}
variant raw-packet-result { pass, modify(raw-packet), drop }
```

::: info Virtual Backend is not in the contract
`server-pre-connect-result` has no `virtual-backend` arm. Virtual Backend is planned and stays deferred to a later minor; the native traits exist but no WASM bridge does. See [Capabilities](./capabilities).
:::

### Command and scheduler callbacks

Commands and scheduled tasks dispatch by a `callback-id` you supply at registration time (`command-manager.register` and `scheduler.delay` / `scheduler.interval`).

```wit
handle-command: func(callback-id: u64, args: list<string>, player: option<player-id>);
tab-complete: func(callback-id: u64, partial: list<string>, cursor: u32) -> list<string>;
on-scheduled-task: func(callback-id: u64);
```

### Marker+proxy callbacks

A `handler-id` is a guest-owned id. When you register a limbo handler, a permission checker, or a codec factory, the host wraps that id in a native marker+proxy object and calls back into the guest through these functions, keying on the id.

```wit
// Limbo handler dispatch: the session is lent by borrow for the call's duration.
limbo-on-player-enter: func(handler: handler-id, session: borrow<limbo-session>) -> handler-result;
limbo-on-command: func(handler: handler-id, session: borrow<limbo-session>, command: string, args: list<string>);
limbo-on-chat: func(handler: handler-id, session: borrow<limbo-session>, message: string);
limbo-on-disconnect: func(handler: handler-id, player: player-id);
limbo-on-session-end: func(handler: handler-id, player: player-id, reason: session-end-reason);

// Custom permission checker dispatch.
permission-level-of: func(handler: handler-id) -> permission-level;
check-permission: func(handler: handler-id, permission: string) -> bool;
```

The `permissions-setup-result::custom(handler-id)` returned from an event tells the host which guest checker to invoke for that player. See [Limbo](./limbo) for the session resources used by the limbo callbacks.

## The codec-filter export

`codec-filter.wit` exports a per-session resource on the hot path. The host calls `create` once per connection and side, then `filter` for every frame, then drops the instance. The `filter` signature takes `(packet-id, data)` only; the codec context is reconstructed guest-side from `codec-session-init` plus the lifecycle calls.

```wit
enum connection-side { client-side, server-side }

record codec-session-init {
    client-version: protocol-version,
    connection-id: u64,
    remote-addr: string,
    real-ip: option<string>,
    side: connection-side,
}

variant codec-filter-error {
    translation-failed(string),
    malformed-payload,
    unsupported-version(s32),
    internal(string),
}

record filter-extras {
    packet: option<raw-packet>,
    inject-before: list<raw-packet>,
    inject-after: list<raw-packet>,
}

variant filter-output {
    pass,
    drop,
    pass-modified(filter-extras),
    replace(filter-extras),
    error(codec-filter-error),
}

resource filter-instance {
    filter: func(packet-id: s32, data: list<u8>) -> filter-output;
    on-state-change: func(new-state: connection-state);
    on-compression-change: func(threshold: s32);
    on-encryption-enabled: func();
    on-close: func();
}

create: func(factory: handler-id, init: codec-session-init) -> filter-instance;
```

The boundary has no `&mut`, so a changed frame is returned in `filter-extras`; `packet none` means unchanged. See [Codec Filters](./codec-filters) for the SDK trait and the chain ordering controlled by `codec-registry`.

## Host service imports

These imports are the host side of the boundary. Methods tagged `host-async` suspend the guest fiber during I/O; methods tagged `gated: X` only link when the plugin holds capability `X`. The author-facing wrappers are on [Services](./services).

### log

Always linked.

```wit
interface log {
    trace: func(message: string);
    debug: func(message: string);
    info: func(message: string);
    warn: func(message: string);
    error: func(message: string);
}
```

### event-bus

```wit
interface event-bus {                                                         // gated: event-bus
    enum event-kind { /* the 17 kinds, matching the event variant arms */ }
    subscribe: func(kind: event-kind, priority: event-priority) -> listener-handle;
    unsubscribe: func(handle: listener-handle);
}
```

### player-registry

The `player` resource holds the per-player methods. `send-packet` is gated by `raw-packet`; `disconnect` and `switch-server` are `host-async`.

```wit
resource player {
    id: func() -> player-id;
    profile: func() -> game-profile;
    protocol-version: func() -> protocol-version;
    remote-addr: func() -> string;
    current-server: func() -> option<server-id>;
    is-connected: func() -> bool;
    is-active: func() -> bool;

    disconnect: func(reason: component);                                  // host-async
    send-message: func(message: component) -> result<_, player-error>;
    send-title: func(title: title-data) -> result<_, player-error>;
    send-action-bar: func(message: component) -> result<_, player-error>;
    send-packet: func(packet: raw-packet) -> result<_, player-error>;     // gated: raw-packet
    switch-server: func(target: server-id) -> result<_, player-error>;    // host-async

    is-online-mode: func() -> bool;
    permission-level: func() -> permission-level;
    has-permission: func(permission: string) -> bool;
    connected-at: func() -> u64;
}

get-player: func(username: string) -> option<player>;
get-player-by-uuid: func(player-uuid: uuid) -> option<player>;
get-player-by-id: func(id: player-id) -> option<player>;
get-players-on-server: func(server: server-id) -> list<player>;
get-all-players: func() -> list<player>;
online-count: func() -> u32;
online-count-on: func(server: server-id) -> u32;
```

### server-manager

```wit
interface server-manager {                                                    // gated: server-manage
    get-state: func(server: server-id) -> option<server-state>;
    start: func(server: server-id) -> result<_, service-error>;              // host-async
    stop: func(server: server-id) -> result<_, service-error>;               // host-async
    get-all-servers: func() -> list<tuple<server-id, server-state>>;
}
```

### ban-service

Every method is `host-async`.

```wit
interface ban-service {                                                       // gated: ban
    ban: func(target: ban-target, reason: option<string>, duration-ms: option<u64>) -> result<_, service-error>;
    unban: func(target: ban-target) -> result<bool, service-error>;
    is-banned: func(target: ban-target) -> result<bool, service-error>;
    get-ban: func(target: ban-target) -> result<option<ban-entry>, service-error>;
    get-all-bans: func() -> result<list<ban-entry>, service-error>;
}
```

### config-service

```wit
interface config-service {                                                    // gated: config-read
    get-server-config: func(server: server-id) -> option<server-config>;
    get-all-server-configs: func() -> list<server-config>;
    get-value: func(key: string) -> option<string>;
}
```

### command-manager

The `callback-id` you pass routes back into `handle-command` and `tab-complete`.

```wit
interface command-manager {                                                   // gated: command
    register: func(name: string, aliases: list<string>, description: string, callback-id: u64);
    unregister: func(name: string);
}
```

### codec-registry

```wit
interface codec-registry {                                                    // gated: codec-filter
    record codec-filter-metadata {
        id: string,
        priority: u8,    // 0=first .. 4=last
        after: list<string>,
        before: list<string>,
    }
    register-codec-filter: func(metadata: codec-filter-metadata, factory: handler-id);
    unregister-codec-filter: func(id: string);
}
```

### scheduler

The `callback-id` routes back into `on-scheduled-task`.

```wit
interface scheduler {                                                         // gated: scheduler
    delay: func(after-ms: u64, callback-id: u64) -> task-handle;
    interval: func(period-ms: u64, callback-id: u64) -> task-handle;
    cancel: func(handle: task-handle);
}
```

### limbo

The host owns the native session and lends it by borrow to the `limbo-on-*` guest callbacks. The capability is enforced when the host mints a session for a capability-holding plugin, not at link time.

```wit
interface limbo {
    variant limbo-entry-context {
        initial-connection(server-id),
        kicked-from-server(tuple<server-id, component>),
        plugin-redirect(option<server-id>),
    }
    variant timeout-outcome {
        accept,
        deny(component),
        redirect(server-id),
        send-to-limbo(list<string>),
    }
    record hold-timeout { after-ms: u64, on-timeout: timeout-outcome }
    variant handler-result {
        accept,
        deny(component),
        hold,
        hold-with-timeout(hold-timeout),
        redirect(server-id),
        send-to-limbo(list<string>),
    }
    enum session-end-reason {
        disconnected, released, kicked, redirected, timed-out, shutdown,
    }

    // Host resource: lent by borrow for the duration of each dispatch call.
    resource limbo-session {
        player-id: func() -> player-id;
        profile: func() -> game-profile;
        entry-context: func() -> limbo-entry-context;
        send-message: func(message: component) -> result<_, player-error>;
        send-title: func(title: title-data) -> result<_, player-error>;
        send-action-bar: func(message: component) -> result<_, player-error>;
        complete: func(outcome: handler-result);
        acquire-handle: func() -> limbo-session-handle;
    }

    // Guest-owned handle: storable across calls, completes a Hold from a callback.
    resource limbo-session-handle {
        player-id: func() -> player-id;
        send-message: func(message: component) -> result<_, player-error>;
        send-title: func(title: title-data) -> result<_, player-error>;
        send-action-bar: func(message: component) -> result<_, player-error>;
        complete: func(outcome: handler-result);
        cancelled: func() -> bool;
    }
    register-limbo-handler: func(name: string, handler: handler-id);
}
```

`acquire-handle` mints an own-able `limbo-session-handle` the guest can store across dispatches and complete later (from `on-scheduled-task` or an event). `cancelled` reports `true` once the engine has ended the session, so a stored handle's scheduled task knows to stop. A `timeout-outcome` is a terminal-only subset of `handler-result`, so a timed-out hold can never re-arm another hold. See [Limbo](./limbo) for the completion model.

## Building against the contract

The SDK embeds `wit-bindgen`, so you compile a plain `cdylib` for `wasm32-wasip2` with no `cargo-component` step.

```bash
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
```

```toml
[lib]
crate-type = ["cdylib"]
```

## See also

- [Events](./events): the typed SDK event surface and which kinds it exposes.
- [Services](./services): wrappers over `player-registry`, `ban-service`, and the other host imports.
- [Limbo](./limbo): the limbo handler and session-completion model.
- [Codec Filters](./codec-filters): the `filter-instance` trait and chain ordering.
- [Capabilities](./capabilities): baseline vs opt-in capability strings and the TOML permissions block.
- [Overview](./): the WASM plugin section landing page.
