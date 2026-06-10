---
title: Server-Only Mode
description: Forwarding mode where the backend handles all authentication while the proxy performs raw TCP relay.
outline: [2, 3]
---

# Server-Only Mode

Server-only mode lets the backend handle all authentication. The proxy forwards raw TCP traffic after the handshake, just like [passthrough](./passthrough.md). The difference is semantic: server-only signals that the backend is responsible for verifying player identities.

## When to use it

Use server-only when your backend handles its own authentication and you want the proxy to stay out of the way. This is the right choice when:

- The backend runs with `online-mode=true` and handles Mojang auth itself
- You need domain-based routing but not packet inspection
- You want the backend to control the full login flow

## Configuration

```toml
name = "my-server"
domains = ["mc.example.com"]
addresses = ["192.168.1.10:25565"]
proxy_mode = "server_only"
```

With Docker labels:

```yaml
labels:
  infrarust.domains: "mc.example.com"
  infrarust.proxy_mode: "server_only"
```

### Config options

| Option | Type | Default | Description |
|---|---|---|---|
| `domains` | string array | required | Domains that route to this server. Supports wildcards (`*.mc.example.com`). |
| `addresses` | string array | required | Backend addresses in `host:port` format. |
| `proxy_mode` | string | `"passthrough"` | Set to `"server_only"`. |
| `send_proxy_protocol` | bool | `false` | Send PROXY protocol header when connecting to the backend. |
| `domain_rewrite` | string or table | `"none"` | Rewrite the hostname in the handshake packet. |
| `max_players` | integer | `0` | Maximum players on this server. `0` means unlimited. |
| `disconnect_message` | string | `"Server is currently unreachable..."` | Message sent to the player when the backend is down. |
| `timeouts.connect` | duration | `"5s"` | How long to wait when connecting to the backend. |
| `timeouts.read` | duration | `"30s"` | Read timeout on the backend connection. |
| `timeouts.write` | duration | `"30s"` | Write timeout on the backend connection. |

::: tip
Duration values use human-readable format: `"5s"`, `"30s"`, `"2m"`, `"1h"`.
:::

## How it works

Server-only uses the same `PassthroughHandler` as passthrough and zero-copy modes. The steps are identical:

1. The proxy reads the client's handshake and login start packets.
2. It fires a `ServerPreConnectEvent`, giving plugins a chance to deny the connection. Redirect results (such as connect-to or limbo) are ignored in forwarding modes.
3. It connects to one of the configured backend addresses.
4. It forwards those initial packets to the backend, applying domain rewrite if configured.
5. It registers a player session with `active: false` (passthrough sessions can't inject packets).
6. It starts bidirectional forwarding via `CopyForwarder`, which calls `tokio::io::copy` in two concurrent tasks.
7. When either side closes, the write half of the other socket is shut down, the remaining bytes drain, and the session ends.

The proxy never decrypts or parses packets after the handshake. The backend receives the raw login sequence and handles Mojang authentication directly.

## Constraints

Server-only is a forwarding mode. The same constraints as passthrough apply:

- At least one domain is required.
- Cannot belong to a network (no server switching).
- No packet injection or inspection. Plugins that send packets to the player won't work.
- Works with every Minecraft version (1.7+).

Domain rewrite still works in server-only. It only affects the initial handshake packet, before the forwarding loop starts.

## Passthrough vs. server-only

Both modes forward raw TCP. The difference is in intent:

| | Passthrough | Server-only |
|---|---|---|
| Who authenticates | Backend (any mode) | Backend (`online-mode=true`) |
| Raw forwarding | Yes | Yes |
| Packet inspection | No | No |
| Implementation | Same handler | Same handler |

In practice, passthrough and server-only behave identically. Server-only exists as a configuration signal that the backend is explicitly expected to handle authentication. Use whichever name makes your config clearer.

## Compared to other modes

- Need lower CPU on Linux? Use [zero-copy](./zerocopy.md). Same forwarding behavior, kernel-level splice.
- Need server switching or plugins? Use [client-only](./client-only.md). The proxy handles Mojang auth and can move players between backends.
- Need server switching without authentication? Use [offline](./offline.md).
