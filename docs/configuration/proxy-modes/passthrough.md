---
title: Passthrough Mode
description: Default proxy mode that forwards raw TCP between client and backend with minimal overhead.
---

# Passthrough Mode

Passthrough is the default proxy mode. It forwards raw TCP traffic between the Minecraft client and your backend server after the handshake, using `tokio::io::copy_bidirectional`. The proxy cannot read, modify, or inject packets in this mode.

## When to use it

Use passthrough when you want the lowest overhead and don't need the proxy to handle authentication, switch servers, or inspect packets. This is the right choice for most single-server setups where the backend handles everything itself.

## Configuration

```toml
name = "my-server"
domains = ["mc.example.com"]
addresses = ["192.168.1.10:25565"]
proxy_mode = "passthrough"
```

Or with Docker labels:

```yaml
labels:
  infrarust.domains: "mc.example.com"
  infrarust.proxy_mode: "passthrough"
```

Since passthrough is the default, you can omit `proxy_mode` entirely and get the same behavior.

## How it works

1. The proxy reads the client's handshake and login start packets.
2. It connects to one of the configured backend addresses.
3. It forwards those initial packets to the backend (applying domain rewrite if configured).
4. It starts a bidirectional byte copy between client and backend.
5. When either side closes the connection, the session ends.

The proxy never decrypts or parses packets after the handshake. The backend handles all authentication, encryption, and game logic.

## Constraints

Passthrough is a forwarding mode. These rules apply:

- You must define at least one domain. The proxy needs a domain to route incoming connections to this server.
- The server cannot belong to a network. Forwarding modes don't support server switching because the proxy can't inject the packets needed to move a player between backends.
- The proxy cannot inject packets into the session. Plugins that need to send chat messages, titles, or other packets to the player won't work.

## Domain rewrite

Even in passthrough mode, you can rewrite the hostname in the handshake packet before it reaches the backend. This is useful when the backend checks the hostname (e.g., for BungeeCord IP forwarding) but the player connects through a different domain.

```toml
name = "my-server"
domains = ["mc.example.com"]
addresses = ["10.0.0.5:25565"]
proxy_mode = "passthrough"
domain_rewrite = { explicit = "backend.internal" }
```

Three rewrite values exist:

| Value | Behavior |
|-------|----------|
| `"none"` | Forward the handshake as-is (default) |
| `{ explicit = "..." }` | Replace the hostname with a specific value |
| `"from_backend"` | Use the backend's address as the hostname |

