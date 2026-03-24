---
title: Server-Only Mode
description: Forwarding mode where the backend handles all authentication while the proxy performs raw TCP relay.
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

Or with Docker labels:

```yaml
labels:
  infrarust.domains: "mc.example.com"
  infrarust.proxy_mode: "server_only"
```

## How it works

Server-only uses the same passthrough handler as passthrough and zero-copy modes. After the handshake, the proxy starts a bidirectional byte copy between client and backend. The backend receives the raw login packets and handles authentication directly with Mojang.

The proxy does not decrypt, parse, or modify any packets beyond the initial handshake.

## Constraints

Server-only is a forwarding mode. The same constraints as passthrough apply:

- At least one domain is required.
- Cannot belong to a network (no server switching).
- No packet injection or inspection.
- Domain rewrite works on the initial handshake.

## Passthrough vs. server-only

Both modes forward raw TCP. The difference is in intent:

| | Passthrough | Server-only |
|---|---|---|
| Who authenticates | Backend (any mode) | Backend (`online-mode=true`) |
| Raw forwarding | Yes | Yes |
| Packet inspection | No | No |
| Implementation | Same handler | Same handler |

In practice, passthrough and server-only behave identically. Server-only exists as a configuration signal that the backend is explicitly expected to handle authentication. Use whichever name makes your config clearer.
