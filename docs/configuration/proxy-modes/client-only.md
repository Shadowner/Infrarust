---
title: Client-Only Mode
description: Proxy handles Mojang authentication while the backend runs in offline mode, enabling packet inspection and server switching.
---

# Client-Only Mode

In client-only mode, the proxy terminates the client's connection and handles Mojang authentication itself. The backend server must run with `online_mode=false` because the proxy, not the backend, verifies the player's identity.

This is the mode you need for server networks, packet inspection, and plugin features that interact with players.

## When to use it

Use client-only when you need any of these:

- Server switching within a network (moving players between backends without reconnecting)
- Packet inspection or modification by plugins
- Centralized authentication across multiple backends
- Features like limbo handlers, codec filters, or event-driven packet injection

## Configuration

```toml
name = "my-server"
domains = ["mc.example.com"]
addresses = ["192.168.1.10:25565"]
proxy_mode = "client_only"
```

Or with Docker labels:

```yaml
labels:
  infrarust.domains: "mc.example.com"
  infrarust.proxy_mode: "client_only"
```

::: danger Backend must be offline
Your backend Minecraft server **must** have `online-mode=false` in its `server.properties`. The proxy already verified the player's identity, so the backend doesn't need to check again. If the backend has `online-mode=true`, players will fail to connect because the backend will try to re-authenticate them.
:::

## How it works

1. The proxy reads the client's handshake and login start packets.
2. It performs Mojang authentication: RSA key exchange, encryption, and session verification against Mojang's servers.
3. On success, it sends `LoginSuccess` to the client.
4. For 1.20.2+, it waits for the client's `LoginAcknowledged` packet and transitions to the Configuration state.
5. It connects to the backend in offline mode, replaying the login sequence.
6. It enters the session loop, parsing and relaying packets in both directions.

Because the proxy parses every packet, sessions are marked as "active." Plugins can inject packets into the stream, and the proxy can move the player to a different backend without dropping the client connection.

## Server networks

Client-only is the required mode for servers that belong to a network. A network lets you group multiple backends and switch players between them.

```toml
name = "hub"
network = "main"
domains = ["network.example.com"]
addresses = ["192.168.1.10:25565"]
proxy_mode = "client_only"

[motd.online]
text = "§6My Network — Hub"
```

```toml
name = "survival"
network = "main"
addresses = ["192.168.1.11:25565"]
proxy_mode = "client_only"
```

The first server has the domain, so it receives incoming connections. Both servers share the `network = "main"` value, so the proxy can move players between them.

::: info
Only intercepted modes (client_only, offline) can belong to a network. Forwarding modes (passthrough, zero_copy, server_only) are rejected during config validation if they specify a network.
:::

