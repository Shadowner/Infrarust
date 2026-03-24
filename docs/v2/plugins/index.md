---
title: Plugins
description: Overview of the Infrarust plugin system — what plugins can do, built-in plugins, and how to build your own.
---

# Plugins

Infrarust ships with a plugin system that lets you hook into the proxy at every stage of a player's connection. Plugins can listen for events, register commands, hold players in limbo screens, filter packets, and access proxy services like the player registry, server manager, and ban system.

Plugins are written in Rust and compiled into the proxy binary. Each plugin implements the `Plugin` trait from `infrarust_api`, declares metadata (id, name, version, dependencies), and receives a `PluginContext` during startup that provides access to everything the proxy offers.

## What plugins can do

**React to events.** The proxy fires events throughout a player's lifecycle: login, disconnect, chat messages, server switches, kicks, and more. A plugin subscribes to these events with a priority level (`FIRST`, `EARLY`, `NORMAL`, `LATE`, `LAST`) and can inspect or modify them before other listeners see them.

**Register commands.** Plugins add proxy-level commands that players can use in chat. Commands get access to the player who ran them and can send messages, switch servers, or trigger other actions.

**Hold players in limbo.** The limbo engine lets plugins intercept players before they reach a backend server. A limbo handler receives the player in a void world and can show titles, send chat messages, and wait for commands before releasing them. This is how the auth plugin implements its login screen and the server-wake plugin holds players while a server boots.

**Filter packets.** Native plugins can register codec-level filters to inspect or modify Minecraft protocol packets as they flow through the proxy. Transport-level filters operate even lower, at the TCP stream level.

**Schedule tasks.** Plugins can run code on a fixed interval using the scheduler, for periodic cleanup, broadcasts, or polling.

**Provide dynamic configuration.** Plugins can register a config provider that supplies server definitions from external sources (databases, APIs, service discovery) instead of static files.

## Built-in plugins

Infrarust includes three built-in plugins, compiled via Cargo feature flags:

| Plugin | Feature flag | Description |
|--------|-------------|-------------|
| [Auth](./builtin/auth) | `plugin-auth` | Password-based authentication with `/login` and `/register` commands. Holds players in limbo until authenticated. |
| [Server Wake](./builtin/server-wake) | `plugin-server-wake` | Holds players in limbo while a backend server starts up, showing status messages. |
| [Queue](./builtin/queue) | `plugin-queue` | Player queue management. (In development.) |

Built-in plugins are registered at compile time in `infrarust-proxy/src/plugins.rs` using a `StaticPluginLoader`. To enable or disable them, toggle the corresponding Cargo feature when building:

```bash
cargo build --release --features "plugin-auth,plugin-server-wake"
```

## Plugin lifecycle

1. **Discovery** — The plugin loader scans for registered plugins and collects their metadata.
2. **Dependency resolution** — Plugins are sorted in dependency order. Required dependencies must be present; optional dependencies adjust ordering when available.
3. **Enable** — `on_enable()` is called on each plugin with a `PluginContext`. This is where plugins register their event listeners, commands, limbo handlers, and scheduled tasks.
4. **Runtime** — The proxy runs. Events flow through registered listeners. Commands are dispatched. Limbo handlers receive players.
5. **Disable** — On shutdown, `on_disable()` is called in reverse dependency order. All resources the plugin registered (listeners, commands, tasks) are automatically cleaned up.

## Next steps

- [Installing Plugins](./installing) — How to enable built-in plugins and configure them.
- [Auth Plugin](./builtin/auth) — Password authentication and limbo login screen.
- [Server Wake Plugin](./builtin/server-wake) — Hold players while backend servers start.
- [Developing Plugins](./dev/getting-started) — Build your own plugin from scratch.
