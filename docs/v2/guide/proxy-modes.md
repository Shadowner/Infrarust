---
title: Proxy Modes
description: Understand the two families of proxy modes in Infrarust and pick the right one for your setup.
outline: [2, 3]
---

# Proxy Modes

When a player connects through Infrarust, the proxy mode controls how traffic flows between the player and your backend server. Every server definition includes a `proxy_mode` setting that tells Infrarust what to do with the connection after the initial handshake.

There are two families of modes: **forwarding** and **intercepted**. The choice between them is the most important decision you'll make when configuring Infrarust.

## Forwarding modes

Forwarding modes relay raw TCP bytes. The proxy reads the handshake packet to figure out which backend to connect to, then copies bytes between the two sockets without touching them. It never decrypts or parses game packets.

Three modes belong to this family:

- **passthrough** (the default) copies bytes in userspace with `tokio::io::copy`.
- **zero_copy** uses the Linux `splice(2)` syscall to move bytes through kernel pipes, avoiding userspace copies entirely.
- **server_only** behaves the same as passthrough. It exists as a config signal that the backend handles authentication with `online-mode=true`.

Because the proxy only reads the handshake (unchanged since Minecraft 1.7), forwarding modes work with every Minecraft version, including modded servers and future releases. The proxy adds almost no overhead to the connection.

```toml
domains = ["survival.mc.example.com"]
addresses = ["10.0.1.10:25565"]
proxy_mode = "passthrough"
```

You don't need to set `proxy_mode` explicitly for passthrough since it's the default.

### When to use forwarding

Use a forwarding mode when you want Infrarust to act as a transparent router. Typical scenarios:

- You run several independent servers behind one public IP and want domain-based routing.
- You don't need the proxy to handle authentication, switch players between servers, or inject packets.
- You're running modded servers or older Minecraft versions that Infrarust's protocol parser doesn't cover.

The tradeoff: forwarding modes cannot inspect or modify packets after the handshake. Plugins that send chat messages, titles, or other packets to the player won't work. Server switching within a network is not possible. Every server must have at least one domain defined.

## Intercepted modes

Intercepted modes terminate the player's connection at the proxy. The proxy handles the login sequence, then opens a separate connection to the backend. Every packet passes through the proxy in both directions, parsed and reassembled.

Two modes belong to this family:

- **client_only** performs Mojang authentication at the proxy. The backend runs with `online-mode=false`.
- **offline** skips authentication. The proxy still parses packets and supports all the same features, but any username can connect.

```toml
domains = ["hub.mc.example.com"]
addresses = ["10.0.1.20:25565"]
proxy_mode = "client_only"
network = "main"
```

::: warning
Intercepted modes require the backend to run with `online-mode=false`. The proxy already authenticated the player (or skipped auth in offline mode), so the backend must not try to re-authenticate.
:::

### When to use intercepted

Use an intercepted mode when you need the proxy to understand what's happening in the session. Typical scenarios:

- You're building a server network where players switch between backends (hub, survival, minigames) without reconnecting.
- You want plugins to interact with players: send messages, modify packets, or run limbo handlers.
- You want centralized Mojang authentication across multiple backend servers.
- You need the proxy to start a backend on demand and hold the player in limbo until it's ready.

The tradeoff: intercepted modes depend on Infrarust's protocol implementation. They work with Minecraft 1.7 through 1.21.x. Future protocol versions require an Infrarust update before they'll work in intercepted mode. Parsing every packet also uses more CPU than raw byte copying.

## Choosing between the two

The decision comes down to one question: does the proxy need to understand the packets?

If no, use a forwarding mode. You get universal version support, minimal overhead, and a proxy that stays out of the way.

If yes, use an intercepted mode. You get server switching, plugin support, and full control over the session, at the cost of version-dependent protocol parsing.

| | Forwarding | Intercepted |
|---|---|---|
| Modes | passthrough, zero_copy, server_only | client_only, offline |
| Packet inspection | No | Yes |
| Server switching | No | Yes |
| Plugin support | No | Yes |
| Minecraft versions | All (1.7+) | 1.7 through 1.21.x |
| Backend `online-mode` | `true` | `false` |
| Requires domain | Yes | Optional (if in a network) |

### Quick decision guide

Start with **passthrough** if you're unsure. It's the default, works everywhere, and adds the least overhead. You can always switch to an intercepted mode later when you need its features.

Pick **client_only** when you need server switching, plugin features, or centralized Mojang auth. This is the mode for server networks.

Pick **offline** when you need the same capabilities as client_only but don't want Mojang authentication. Useful for cracked servers or local development.

Pick **zero_copy** if you're on Linux and want to squeeze out lower CPU usage on a high-traffic forwarding setup.

## Example: single server with domain routing

A standalone survival server behind Infrarust. No network, no plugins, just domain-based routing:

```toml
domains = ["survival.mc.example.com"]
addresses = ["192.168.1.10:25565"]
```

Passthrough is the default. The proxy reads the handshake, connects to the backend, and copies bytes.

## Example: server network with switching

A hub and two game servers. Players connect to the hub domain and get moved between servers:

```toml
name = "hub"
network = "main"
domains = ["play.mc.example.com"]
addresses = ["192.168.1.10:25565"]
proxy_mode = "client_only"
```

```toml
name = "survival"
network = "main"
addresses = ["192.168.1.11:25565"]
proxy_mode = "client_only"
```

```toml
name = "minigames"
network = "main"
addresses = ["192.168.1.12:25565"]
proxy_mode = "client_only"
```

The hub has the domain, so it receives incoming connections. The other two servers have no domains; players reach them through server switching. All three share `network = "main"`.

::: tip
Only the entry point server in a network needs a domain. Other servers can omit `domains` entirely if players only reach them through server switching.
:::

## Next steps

- [Passthrough configuration](../configuration/proxy-modes/passthrough.md) for detailed options and behavior
- [Zero-copy configuration](../configuration/proxy-modes/zerocopy.md) for Linux kernel-level forwarding
- [Client-only configuration](../configuration/proxy-modes/client-only.md) for Mojang auth and server networks
- [Offline configuration](../configuration/proxy-modes/offline.md) for no-auth setups
- [Proxy modes overview](../configuration/proxy-modes/) for the full comparison table
