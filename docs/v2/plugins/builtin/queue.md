---
title: Queue Plugin
description: Planned plugin for holding players in a wait room when a backend server is full
---

# Queue Plugin

::: warning In development
The queue plugin is not yet implemented. The `infrarust-plugin-queue` crate exists as a placeholder with no functional code. Do not configure it expecting results; the plugin currently does nothing.
:::

When it ships, the queue plugin will hold players in a limbo wait room when a backend server has reached its player limit. Instead of getting a "server is full" kick, a connecting player enters a proxy-managed screen showing their position in line and an estimated wait time. When a slot opens up, the player at the front of the queue gets forwarded automatically.

## Intended behavior

The general flow will work like this:

1. A player connects to a domain whose backend reports a full server (at or above `max_players`).
2. The plugin intercepts the connection before it reaches the backend and places the player in limbo.
3. The player sees their queue position and a message that updates as the queue moves.
4. When a slot becomes available, the plugin forwards the first player in line and shifts everyone else up.
5. If a queued player disconnects, they leave the queue and their position is not held.

This is built on the same limbo mechanism that the [auth plugin](./auth) uses for login screens and the [server wake plugin](./server-wake) uses for startup screens. See [Limbo Handlers](../wasm/limbo) for how that mechanism works.

## Current state

The crate is a stub. Its `src/lib.rs` is a single placeholder comment and its `Cargo.toml` depends only on `infrarust-api`. There is no plugin struct, no `Plugin` implementation, no queue logic, no configuration, and no limbo handler registered. The crate is not yet wired into the workspace build.

If you need wait-room behavior today, a [WASM plugin](../wasm/getting-started) implementing a limbo handler is the practical path. A handler can read a backend's configured `max_players` and the set of connected players, then hold or forward each connecting player based on current occupancy. See [services](../wasm/services) for the data a plugin can read.
