---
title: Offline Mode
description: No-authentication proxy mode with full packet access, for cracked servers and local development.
---

# Offline Mode

Offline mode skips all authentication. The proxy does not verify the player's identity with Mojang. It still parses every packet, so plugins and server switching work the same as in [client-only mode](./client-only.md).

## When to use it

Use offline mode when:

- Your server allows cracked (non-premium) clients
- You're developing locally and don't want to deal with Mojang auth
- You want packet inspection and server switching but don't need identity verification

## Configuration

```toml
name = "my-server"
domains = ["mc.example.com"]
addresses = ["192.168.1.10:25565"]
proxy_mode = "offline"
```

Or with Docker labels:

```yaml
labels:
  infrarust.domains: "mc.example.com"
  infrarust.proxy_mode: "offline"
```

## How it works

1. The proxy reads the client's handshake and login start packets.
2. It fires the `PreLoginEvent` (plugins can deny the connection here).
3. It fires the `PostLoginEvent`.
4. It connects to the backend and replays the login sequence.
5. It enters the session loop, parsing and relaying packets in both directions.

No RSA key exchange happens. No `LoginSuccess` is sent to the client during the proxy's auth phase. The backend handles the login completion.

Like client-only, sessions are marked as "active," so plugins can inject packets and the proxy can switch the player between servers.

## Differences from client-only

| | Client-only | Offline |
|---|---|---|
| Mojang authentication | Yes | No |
| `LoginSuccess` sent by proxy | Yes | No |
| Packet inspection | Yes | Yes |
| Server switching | Yes | Yes |
| Cracked clients | No | Yes |

## Constraints

Offline mode is an intercepted mode with no forwarding-mode restrictions. It can belong to a network and supports server switching. The only constraint is that at least one backend address must be configured.

::: warning No identity verification
Without Mojang authentication, anyone can connect with any username. If your server is public-facing, consider using a plugin to handle authentication (e.g., AuthMe on the backend) or use client-only mode instead.
:::

