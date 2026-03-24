---
title: Passthrough Mode
description: Default proxy mode that forwards raw TCP between client and backend with minimal overhead.
outline: [2, 3]
---

# Passthrough Mode

Passthrough is the default proxy mode. It forwards raw TCP traffic between the Minecraft client and your backend server after the handshake. The proxy never decrypts or parses game packets in this mode.

## When to use it

Pick passthrough when you want the lowest overhead and don't need the proxy to handle authentication, switch servers, or inspect packets. This covers most single-server setups where the backend handles everything itself.

## Configuration

A minimal server config file:

```toml
domains = ["mc.example.com"]
addresses = ["192.168.1.10:25565"]
```

Since passthrough is the default `proxy_mode`, you don't need to set it explicitly. If you want to be explicit:

```toml
domains = ["mc.example.com"]
addresses = ["192.168.1.10:25565"]
proxy_mode = "passthrough" // [!code focus]
```

With Docker labels:

```yaml
labels:
  infrarust.domains: "mc.example.com"
  infrarust.proxy_mode: "passthrough"
```

### Full example

This uses every option that applies to passthrough:

```toml
name = "survival"
domains = ["survival.mc.example.com", "*.survival.example.com"]
addresses = ["10.0.1.10:25565", "10.0.1.11:25565"]
proxy_mode = "passthrough"
send_proxy_protocol = false
domain_rewrite = "none"
max_players = 100
disconnect_message = "Survival server is offline. Try again later."

[timeouts]
connect = "3s"
read = "30s"
write = "30s"
```

### Config options

| Option | Type | Default | Description |
|---|---|---|---|
| `domains` | string array | required | Domains that route to this server. Supports wildcards (`*.mc.example.com`). |
| `addresses` | string array | required | Backend addresses in `host:port` format. |
| `proxy_mode` | string | `"passthrough"` | Set to `"passthrough"` or omit entirely. |
| `send_proxy_protocol` | bool | `false` | Send PROXY protocol header when connecting to the backend. |
| `domain_rewrite` | string or table | `"none"` | Rewrite the hostname in the handshake packet. See [Domain rewrite](#domain-rewrite). |
| `max_players` | integer | `0` | Maximum players on this server. `0` means unlimited. |
| `disconnect_message` | string | `"Server is currently unreachable..."` | Message sent to the player when the backend is down. |
| `timeouts.connect` | duration | `"5s"` | How long to wait when connecting to the backend. |
| `timeouts.read` | duration | `"30s"` | Read timeout on the backend connection. |
| `timeouts.write` | duration | `"30s"` | Write timeout on the backend connection. |

::: tip
Duration values use human-readable format: `"5s"`, `"30s"`, `"2m"`, `"1h"`.
:::

## How it works

1. The proxy reads the client's handshake and login start packets.
2. It fires a `ServerPreConnectEvent`, giving plugins a chance to deny or redirect the connection.
3. It connects to one of the configured backend addresses.
4. It forwards those initial packets to the backend, applying domain rewrite if configured.
5. It registers a player session (with `active: false`, since passthrough can't inject packets).
6. It starts two concurrent tasks: one copies bytes from client to backend, the other from backend to client. Both run through `tokio::io::copy`.
7. When either side closes the connection, the write half of the other socket is shut down, the remaining bytes drain, and the session ends.

The proxy never decrypts or parses packets after the handshake. The backend handles all authentication, encryption, and game logic.

### Connection lifecycle

```
Client ──TCP──▶ Infrarust ──TCP──▶ Backend
                   │
         reads handshake + login start
                   │
         connects to backend
                   │
         forwards initial packets
                   │
         starts bidirectional copy ─────────▶ raw bytes flow
                   │                          both directions
         waits for either side to close
                   │
         session ends, cleanup
```

## Performance

Passthrough uses `CopyForwarder`, which calls `tokio::io::copy` in userspace. Each direction gets its own tokio task, so both run concurrently on the async runtime.

There are no configurable buffer sizes. Tokio manages the internal buffers. For most Minecraft workloads, this adds negligible overhead compared to a direct connection.

If you're running on Linux and want to skip the userspace copy entirely, consider [zero-copy mode](./zerocopy.md). It uses the `splice(2)` syscall to move bytes through kernel pipes without copying them into your process. This can reduce CPU usage on high-traffic proxies.

::: info
Passthrough tracks bytes transferred in both directions. You can see the totals in the session end log:

```
session ended c2b=1048576 b2c=8388608 reason=ClientClosed
```
:::

## Constraints

Passthrough is a forwarding mode. These rules apply:

- You must define at least one domain. The proxy needs a domain to route incoming connections to this server.
- The server cannot belong to a network. Forwarding modes don't support server switching because the proxy can't inject the transfer packets needed to move a player between backends.
- The proxy cannot inject packets into the session. Plugins that need to send chat messages, titles, or other packets to the player won't work.
- Works with every Minecraft version (1.7+). The proxy only reads the handshake packet, which hasn't changed since 1.7.

## Domain rewrite

You can rewrite the hostname in the handshake packet before it reaches the backend. This is useful when the backend checks the hostname (for BungeeCord IP forwarding, for example) but the player connects through a different domain.

```toml
domains = ["mc.example.com"]
addresses = ["10.0.0.5:25565"]
domain_rewrite = { explicit = "backend.internal" }
```

Three rewrite strategies:

| Value | Behavior |
|---|---|
| `"none"` | Forward the handshake as-is (default) |
| `{ explicit = "..." }` | Replace the hostname with a specific value |
| `"from_backend"` | Use the first backend address as the hostname |

When using `from_backend`, the proxy takes the `host` part of the first entry in `addresses`. If you have `addresses = ["10.0.0.5:25565"]`, the handshake hostname becomes `10.0.0.5`.

## PROXY protocol

If your backend needs the real client IP (and supports PROXY protocol), enable it:

```toml
domains = ["mc.example.com"]
addresses = ["10.0.0.5:25565"]
send_proxy_protocol = true
```

The proxy sends a PROXY protocol header when it opens the backend connection, before forwarding the handshake. Your backend must be configured to accept PROXY protocol connections.

## Compared to other modes

If passthrough doesn't fit your needs:

- Need lower CPU on Linux? Use [zero-copy](./zerocopy.md). Same behavior, kernel-level forwarding.
- Need server switching or plugins? Use [client-only](./client-only.md). The proxy handles Mojang auth and can move players between backends.
- Need server switching without authentication? Use [offline](./offline.md).
