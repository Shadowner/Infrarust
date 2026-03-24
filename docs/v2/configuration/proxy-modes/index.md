---
title: Proxy Modes
description: How Infrarust handles the connection between players and your backend servers, from raw TCP forwarding to full packet interception.
outline: [2, 3]
---

# Proxy Modes

Every server you configure in Infrarust has a proxy mode that controls what the proxy does with traffic between the player and your backend. The mode you pick determines whether the proxy can read packets, handle authentication, or switch players between servers.

## Two categories

Proxy modes fall into two groups: **forwarding** and **intercepted**.

Forwarding modes relay raw TCP bytes after the initial handshake. The proxy never decrypts or parses game packets. Intercepted modes terminate the client connection at the proxy, parse every packet, and open a separate connection to the backend.

| | Forwarding | Intercepted |
|---|---|---|
| Modes | passthrough, zero_copy, server_only | client_only, offline |
| Packet inspection | No | Yes |
| Server switching | No | Yes |
| Plugin packet injection | No | Yes |
| Can join a network | No | Yes |

## Comparison table

Here is a side-by-side breakdown of all five modes.

| Feature | passthrough | zero_copy | server_only | client_only | offline |
|---|---|---|---|---|---|
| Category | Forwarding | Forwarding | Forwarding | Intercepted | Intercepted |
| Default | Yes | No | No | No | No |
| Parses packets | No | No | No | Yes | Yes |
| Auth handled by | Backend | Backend | Backend | Proxy (Mojang) | None |
| Backend `online-mode` | `true` | `true` | `true` | `false` | `false` |
| Server switching | No | No | No | Yes | Yes |
| Plugin packet injection | No | No | No | Yes | Yes |
| Can join a network | No | No | No | Yes | Yes |
| Requires domain | Yes | Yes | Yes | No (if in network) | No (if in network) |
| OS restriction | None | Linux only | None | None | None |
| Minecraft version support | All (1.7+) | All (1.7+) | All (1.7+) | Supported versions | Supported versions |

::: tip
Forwarding modes work with every Minecraft version because they only read the handshake packet. Intercepted modes depend on Infrarust's protocol implementation (currently 1.7 through 1.21.x).
:::

## Forwarding modes

Forwarding modes work with every Minecraft version, past and future. Because the proxy only reads the handshake packet (which hasn't changed since Minecraft 1.7), it doesn't need to understand the game protocol at all. The rest is raw bytes moving between two sockets.

This makes forwarding modes the right choice when you want to expose a single port and route players to different backend servers based on the domain they connect with. You can run a 1.8 PvP server, a 1.20 survival server, and a modded 1.12.2 server all behind one Infrarust instance, each on its own domain. The proxy routes the connection and gets out of the way.

- [Passthrough](./passthrough.md) is the default. It uses `tokio::io::copy_bidirectional` to relay bytes in userspace. Works on every OS.
- [Zero-copy](./zerocopy.md) uses the Linux `splice(2)` syscall to move bytes through kernel pipes without copying them into userspace. Lower CPU usage on busy proxies. Falls back to passthrough on non-Linux systems.
- [Server-only](./server-only.md) is functionally identical to passthrough. It exists as a config signal that the backend is expected to handle authentication with `online-mode=true`.

All three share the same constraints: you must define at least one domain, the server cannot belong to a network, and the proxy cannot inject packets.

## Intercepted modes

Intercepted modes parse the Minecraft protocol. The proxy terminates the player's connection, handles the login sequence, then opens a separate connection to the backend. This gives the proxy full control over the session: it can read and modify packets, move the player to a different backend, and let plugins interact with the player.

The tradeoff is that intercepted modes depend on Infrarust's protocol support. They work with the Minecraft versions that the proxy knows how to parse (currently 1.7 through 1.21.x).

- [Client-only](./client-only.md) performs Mojang authentication at the proxy. The backend must run with `online-mode=false`. This is the mode you need for server networks where players switch between backends without reconnecting.
- [Offline](./offline.md) skips authentication entirely. The proxy still parses packets and supports server switching, but any username can connect. Use this for cracked servers or local development.

## Decision flowchart

Use these questions to pick the right mode:

```
Do you need server switching or plugin packet injection?
├── No
│   ├── Running on Linux and want lower CPU usage?
│   │   ├── Yes → zero_copy
│   │   └── No → passthrough (default)
│   └── Backend handles its own auth and you want to signal that?
│       └── Yes → server_only
└── Yes
    ├── Do players need Mojang authentication (premium accounts)?
    │   ├── Yes → client_only
    │   └── No → offline
```

Start with `passthrough` if you're unsure. Move to `client_only` when you need server switching or plugins. Move to `zero_copy` if you're on Linux and want to reduce CPU overhead on a high-traffic proxy.

## Quick reference

| You want to... | Use |
|---|---|
| Route by domain, minimal overhead | `passthrough` |
| Route by domain, lower CPU on Linux | `zero_copy` |
| Backend handles auth, proxy just routes | `server_only` |
| Proxy handles auth, server switching, plugins | `client_only` |
| No auth, server switching, plugins | `offline` |

## Configuration

Set the mode in your server config file:

```toml
proxy_mode = "passthrough"
```

Valid values: `passthrough`, `zero_copy`, `client_only`, `offline`, `server_only`.

The default is `passthrough`.

With Docker labels:

```yaml
labels:
  infrarust.proxy_mode: "client_only"
```

::: warning
`zero_copy` only works on Linux. On other operating systems, the proxy logs a warning and falls back to passthrough behavior.
:::
