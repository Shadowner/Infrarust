---
title: FAQ
description: Answers to frequent questions about Infrarust, including supported Minecraft versions, player limits, resource usage, and how it compares to Velocity and BungeeCord.
---

# FAQ

## Which Minecraft versions does Infrarust support?

It depends on the proxy mode.

**Forwarding modes** (passthrough, zero_copy, server_only) work with every Minecraft version from 1.7 onward, including snapshots, modded servers, and future releases. The proxy only reads the handshake packet, which hasn't changed since 1.7. It never parses game packets, so the client version doesn't matter.

**Intercepted modes** (client_only, offline) parse and re-encode game packets, so they depend on Infrarust's protocol implementation. The protocol crate currently defines versions from 1.7.2 (protocol 4) through 1.21.11 (protocol 774). The limbo system handles version-specific spawn sequences for clients as old as pre-1.16.

Legacy clients (Beta 1.8 through 1.6) are detected during the handshake and handled separately. They receive a legacy ping response but cannot log in through the proxy.

::: tip
If you're unsure, use passthrough mode. It works with any Minecraft version and adds the least overhead.
:::

## Is there a maximum player count?

No hard-coded limit. The `max_players` field in a server config controls per-server caps, and `max_connections` in `infrarust.toml` sets a global connection limit. Both default to 0 (unlimited).

```toml
# infrarust.toml — global limit
max_connections = 500
```

```toml
# servers/survival.toml — per-server limit
max_players = 100
```

In practice, the limit comes from your OS (file descriptors, TCP buffers) and the backend servers themselves. Infrarust's forwarding modes use very little CPU per connection because they copy raw bytes without parsing.

## How much CPU and memory does Infrarust use?

Forwarding modes (passthrough, zero_copy) add minimal overhead. The proxy copies TCP streams without decrypting or parsing packets. On Linux, zero_copy mode uses the `splice(2)` syscall to move data through kernel pipes, avoiding userspace copies entirely.

Intercepted modes (client_only, offline) use more CPU because every packet passes through the proxy's codec. The exact cost depends on the number of active players and how much traffic flows through the connection.

Memory usage scales with active connections. Each connection holds a TCP stream, a small context struct, and (in intercepted mode) packet buffers. The proxy itself has no world data, chunk storage, or entity tracking.

You can control the number of tokio worker threads with the `worker_threads` setting in `infrarust.toml`. The default (0) lets tokio choose based on available CPU cores.

## How does Infrarust compare to Velocity?

Velocity is a Minecraft proxy written in Java. It terminates the Minecraft protocol, handles authentication, and supports server switching through its own plugin API. Velocity operates in what Infrarust calls "intercepted" mode: it parses every packet flowing between the client and backend.

Infrarust does the same thing in its client_only and offline modes. The main differences:

- Infrarust is written in Rust and runs as a single static binary. No JVM, no Java runtime.
- Infrarust offers forwarding modes (passthrough, zero_copy) that Velocity does not. These modes relay raw bytes without parsing, which means lower overhead and version-independent operation.
- Velocity has a mature plugin ecosystem with community support. Infrarust's plugin system is newer and has fewer third-party plugins.
- Velocity supports player info forwarding to backends (modern forwarding). Infrarust handles authentication at the proxy and tells backends to run with `online-mode=false`.

If you need a transparent TCP proxy with domain routing and don't need packet inspection, Infrarust's forwarding modes have no equivalent in Velocity.

If you need a full protocol-aware proxy with a large plugin ecosystem, Velocity is more mature in that area.

## How does Infrarust compare to BungeeCord?

BungeeCord is the original Minecraft proxy, also written in Java. It predates Velocity and serves a similar role: protocol termination, authentication, and server switching.

BungeeCord has known limitations that both Velocity and Infrarust address:

- BungeeCord's IP forwarding relies on a handshake hack that any client can spoof if the backend's firewall isn't locked down. Infrarust's client_only mode uses standard Mojang authentication at the proxy.
- BungeeCord does not support modern Minecraft protocol features as quickly as Velocity. Its plugin API is older and less actively maintained.
- BungeeCord has no equivalent to Infrarust's forwarding modes.

For new setups, Velocity or Infrarust are better choices than BungeeCord.

## Can I run Infrarust in front of Velocity or BungeeCord?

Yes. Use passthrough mode. Infrarust handles domain routing and forwards the raw connection to Velocity or BungeeCord, which then handles authentication and server switching. The downstream proxy sees the connection as if the player connected directly.

```toml
domains = ["play.mc.example.com"]
addresses = ["10.0.1.5:25577"]
proxy_mode = "passthrough"
```

If you need the downstream proxy to see the player's real IP, enable proxy protocol on both sides:

```toml
domains = ["play.mc.example.com"]
addresses = ["10.0.1.5:25577"]
proxy_mode = "passthrough"
send_proxy_protocol = true
```

## Does Infrarust support modded servers (Forge, Fabric, NeoForge)?

In forwarding modes, yes. The proxy strips Forge Mod Loader markers (`\0FML\0`, `\0FML2\0`, `\0FML3\0`) from the handshake domain for routing purposes, then forwards the original bytes unchanged to the backend. The mod handshake happens directly between the client and the backend server.

In intercepted modes, modded servers work as long as the Minecraft protocol version is supported. Custom packets from mods pass through the proxy's codec as opaque plugin channel messages.

## Does Infrarust support Bedrock Edition?

No. Infrarust handles the Minecraft Java Edition protocol only. Bedrock uses a different network protocol (RakNet) that Infrarust does not implement.

## Can players switch between servers without reconnecting?

Yes, but only in intercepted modes (client_only, offline). Servers must share the same `network` value in their config. A plugin can then move a player from one backend to another while the proxy maintains the client connection.

Forwarding modes cannot switch servers because the proxy doesn't parse packets after the handshake.

## What license is Infrarust released under?

AGPL-3.0. The source code is available on [GitHub](https://github.com/Shadowner/Infrarust).
