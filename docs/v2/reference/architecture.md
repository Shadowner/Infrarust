---
title: Architecture Overview
description: Internal structure of Infrarust v2. The workspace crate layout, the connection pipeline and middleware order, proxy-mode handlers, and the shared services and registries that hold proxy state.
---

# Architecture Overview

This page describes how Infrarust is put together internally: the crates that make up the workspace, the order in which middleware runs on each connection, the handlers that take over once routing is done, and the shared services that hold proxy state. It is meant for contributors and plugin authors who want to know where a given concern lives in the code.

Everything here reflects version `2.0.0-beta.3` on the main branch. Infrarust is a Rust 2024 workspace with an MSRV of 1.94.

## Crate layout

Infrarust is split into a set of focused crates under `crates/`. Each one owns a single concern, which keeps the dependency graph shallow and makes the protocol and config layers reusable on their own.

| Crate | Role |
|-------|------|
| `infrarust` | The binary. CLI parsing, process startup, and wiring the runtime together. |
| `infrarust-core` | The proxy engine: connection pipeline, middleware, handlers, routing, services, and the event bus. |
| `infrarust_config` | Config types and parsing for `infrarust.toml` and per-server TOML, with defaults. |
| `infrarust_protocol` | Minecraft packet definitions, the VarInt/VarLong codec, version tables, and the packet registry. |
| `infrarust-api` | The stable surface plugins build against: traits, event types, service interfaces, and the permission model. |
| `infrarust-transport` | The listener, the backend connector, keepalive, and PROXY protocol handling. |
| `infrarust_server_manager` | Lifecycle control for managed backends (local process, Pterodactyl, Crafty). |
| `infrarust-loader-wasm` | The host-side WebAssembly plugin loader built on wasmtime. |
| `infrarust-plugin-wit` | The WIT interface contract shared by the host and guest plugins. |
| `infrarust-plugin-sdk` | The guest-side SDK that plugin authors compile against. |

Two more crates round out the plugin tooling: `infrarust-plugin-macros` provides the proc-macros used by the SDK, and the workspace also vendors a handful of first-party plugins under `plugins/`.

The dependency direction is one-way. `infrarust_protocol` and `infrarust_config` sit at the bottom and depend on neither each other nor the core. `infrarust-api` defines the plugin contract without pulling in the engine, so a plugin compiles against the API alone. `infrarust-core` depends on all of the above and is in turn consumed only by the `infrarust` binary and the loaders.

The WASM toolchain talks across a frozen contract rather than a Rust dependency. The host loader and the guest SDK both speak the WIT world declared in `crates/infrarust-plugin-wit/wit/world.wit`, currently `infrarust:plugin@0.3.0`. Guest plugins build for the `wasm32-wasip2` target as a `cdylib`, with no separate cargo-component step.

## Connection lifecycle

A connection is handled by `ProxyServer`, built in `ProxyServer::new` and driven by `ProxyServer::run` (`crates/infrarust-core/src/server.rs`). Startup loads server configs through the provider registry, builds the status, ban, and forwarding subsystems, assembles the shared services, and then constructs two pipelines and the per-mode handlers. After that, `run` binds the listener and enters the accept loop.

For each accepted socket the server spawns a task. The accept loop never reads from the socket: the task reads the PROXY protocol header when `receive_proxy_protocol` is on, runs the transport filters, and then runs the connection through the common pipeline. Once the handshake is parsed, the connection branches on its declared intent.

A status ping (`ConnectionIntent::Status`) goes straight to the status handler, which answers from the status cache or relays a live request to the backend. It never enters the login pipeline.

A login (`ConnectionIntent::Login`) runs through the login pipeline and is then dispatched to a handler chosen by the resolved server's `proxy_mode`. The transfer intent is parsed but not yet handled.

Pipelines are sequential. A `Pipeline` (`crates/infrarust-core/src/pipeline/mod.rs`) holds a `Vec<Box<dyn Middleware>>` and runs each middleware in insertion order, stopping at the first non-continue result. Each middleware returns one of three outcomes defined by `MiddlewareResult`: `Continue` moves to the next middleware, `ShortCircuit` ends pipeline processing (for example when a legacy ping is detected and answered inline), and `Reject(String)` closes the connection, normally after sending a kick packet with the given reason.

State accumulates in a `ConnectionContext` that flows through both pipelines. Middleware reads and writes typed entries in the context's extension map: the handshake parser inserts `HandshakeData`, the domain router inserts `RoutingData`, and the login-start parser inserts `LoginData`. A handler downstream pulls those back out by type. These types live in `crates/infrarust-core/src/pipeline/types.rs`.

## Middleware order

Two pipelines run on every login connection, in this order.

The common pipeline runs on every connection regardless of intent:

1. `IpFilter` checks the peer address against the configured allow and deny rules.
2. `HandshakeParser` reads the handshake packet, detects legacy pings, strips FML markers from the domain, and records the protocol version and intent.
3. `BanIpCheck` refuses status connections from IP-banned addresses. Login connections pass through to `BanCheck` in the login pipeline.
4. `RateLimiter` applies the connection rate limit per source address.
5. `DomainRouter` resolves the requested domain to a server config and inserts `RoutingData`.

The login pipeline runs only after a connection is identified as a login:

1. `LoginStartParser` reads the username and, on 1.20.2 and newer, the player UUID.
2. `BanCheck` rejects banned players by name or UUID.
3. `Telemetry` opens the per-connection tracing span. Login connections are always traced at full sampling.
4. `BackendSelection` orders the routed server's backend addresses for the handler.

Waking a managed server is not a pipeline step. The handlers wake it once the plugins have chosen the server the player connects to, after `ServerPreConnectEvent`, so a player refused at login or sent to another server never starts it. See [server wake](../plugins/dev/events#server-wake).

The order matters. The IP filter is the only check that runs before the handshake is parsed, so a filtered peer is dropped before the proxy spends work decoding its packets. The IP ban check needs the intent from the handshake, parsing runs before rate limiting and routing, and player-identity checks run only once routing has chosen a server.

## Proxy mode handlers

After the login pipeline passes, the server reads the resolved `proxy_mode` and hands the connection to one of three handlers. The `ProxyMode` enum lives in `crates/infrarust_config/src/types/proxy_mode.rs` and is serialized in snake_case.

The forwarding family (`passthrough`, the default, plus `zero_copy` and `server_only`) does raw byte forwarding after the handshake with no packet inspection. These run through the passthrough handler. `passthrough` copies bytes with `tokio::io::copy_bidirectional`; `zero_copy` uses the Linux `splice(2)` path; `server_only` forwards while leaving authentication to the backend.

The intercepted family (`client_only` and `offline`) parses packets, which is what makes server switching and limbo possible. `offline` and `client_only` each have their own configured `InterceptedHandler`. `client_only` runs Mojang authentication on the proxy side and expects the backend in offline mode; `offline` is a transparent relay with no authentication.

::: info
Multiple backend `addresses` on a server are ordered by that server's balancing strategy, and failing addresses are ejected until they answer again. See [Load balancing](../configuration/load-balancing).
:::

## Services and registries

`ProxyServices` (`crates/infrarust-core/src/services/proxy.rs`) is the bag of shared state created once in `ProxyServer::new` and cloned per connection. Every field is an `Arc`, so cloning is cheap and the underlying state is shared, not copied. This is also the object handlers and plugins reach through to touch proxy-wide state.

The registries inside it each track one kind of thing:

- The connection registry tracks active player sessions and is what the player-count subcommands and the server manager read.
- The player registry resolves connected players by name or UUID on top of the connection registry.
- The packet registry holds packet codecs keyed by protocol version. Infrarust supports 36 Minecraft versions, from 1.7.2 (protocol 4) to 26.2 (protocol 776), defined in `crates/infrarust_protocol/src/version.rs`. Each packet type declares its own per-version ID table via `Packet::IDS`; the registry indexes those tables rather than owning them.
- The command manager registers and dispatches the in-game `/infrarust` subcommands.
- The codec filter registry builds the per-connection codec filter chain used by intercepted modes.
- The transport filter registry keeps the transport filter chain, rebuilt whenever a filter is added or removed. Each connection's task runs the chain after the PROXY protocol header and before the connection pipeline; it can reject the peer at the socket level, and the filters that accepted the connection are closed when the connection context is dropped.
- The limbo handler registry holds named limbo handler instances.

Both filter registries tag every filter with its owner, the proxy or a plugin id. A plugin can only replace or remove its own filters, and the plugin's filters are removed when it is disabled.

Alongside the registries, `ProxyServices` carries the event bus, the ban manager, the domain router, the optional server manager, the active forwarding mode and its secret, the permission service, and the parsed `ProxyConfig`.

Server configs are not loaded directly by the server. A provider registry (`crates/infrarust-core/src/provider/`) owns the file provider, which is always on, and an optional Docker provider behind a feature flag. Providers load the initial configs into the domain router and start watchers that push live updates, which is how config hot-reload works without restarting the proxy.

Permissions are string nodes such as `infrarust.admin` or `infrarust.command.kick`. One `PermissionProvider` is active, the built-in one or the plugin named in `[permissions] provider`, and it builds a `PermissionChecker` per player that answers each node with a `Tristate` (`True`, `False` or `Undefined`). An `Undefined` answer falls back to the default the node was registered with (`True`, `False` or `Admin`), then to denied. `PermissionMap` holds node values with wildcard matching for providers that want it. The types live in `crates/infrarust-api/src/permissions.rs` and the permission service in `crates/infrarust-core/src/permissions/`. There are no permission levels: an admin is a player who holds `infrarust.admin`.

## Where to go next

For how packets are forwarded once a handler takes over, see [Zerocopy & Splice](./zerocopy). For the configuration fields named above, see the [Config Schema](./config-schema). The WASM plugin contract and its host loader are covered under the [plugin documentation](/plugins/wasm/).
