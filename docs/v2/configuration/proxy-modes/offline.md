---
title: Offline Mode
description: No-authentication proxy mode with full packet parsing for cracked servers, local development, and custom authentication plugins.
outline: [2, 3]
---

# Offline Mode

Offline mode skips Mojang authentication entirely. The proxy does not verify the player's identity. It still parses every packet, so plugins, server switching, and limbo handlers all work the same as in [client-only mode](./client-only.md).

## When to use it

Use offline mode when:

- Your server allows cracked (non-premium) clients
- You're developing locally and don't want to deal with Mojang auth
- You want packet inspection and server switching without identity verification
- You handle authentication yourself through a plugin (AuthMe, etc.)

## Configuration

A minimal server config file:

```toml
domains = ["mc.example.com"]
addresses = ["192.168.1.10:25565"]
proxy_mode = "offline"
```

With Docker labels:

```yaml
labels:
  infrarust.domains: "mc.example.com"
  infrarust.proxy_mode: "offline"
```

### Full example

This uses every option that applies to offline mode:

```toml
name = "lobby"
network = "main"
domains = ["play.mc.example.com", "*.mc.example.com"]
addresses = ["10.0.1.10:25565", "10.0.1.11:25565"]
proxy_mode = "offline"
send_proxy_protocol = false
max_players = 200
disconnect_message = "Server is offline. Try again later."
limbo_handlers = ["motd", "tablist"]

[timeouts]
connect = "3s"
read = "30s"
write = "30s"

[motd.online]
text = "§6Lobby — Online"
```

### Config options

| Option | Type | Default | Description |
|---|---|---|---|
| `domains` | string array | `[]` | Domains that route to this server. Supports wildcards (`*.mc.example.com`). Can be empty if the server is only reachable via server switching within a network. |
| `addresses` | string array | required | Backend addresses in `host:port` format. |
| `proxy_mode` | string | `"passthrough"` | Set to `"offline"`. |
| `network` | string | none | Network name. Servers in the same network can switch players between each other. |
| `send_proxy_protocol` | bool | `false` | Send PROXY protocol header when connecting to the backend. |
| `max_players` | integer | `0` | Maximum players on this server. `0` means unlimited. |
| `disconnect_message` | string | `"Server is currently unreachable..."` | Message sent to the player when the backend is down. |
| `limbo_handlers` | string array | `[]` | Plugin IDs for limbo handler chain, executed in order. |
| `timeouts.connect` | duration | `"5s"` | How long to wait when connecting to the backend. |
| `timeouts.read` | duration | `"30s"` | Read timeout on the backend connection. |
| `timeouts.write` | duration | `"30s"` | Write timeout on the backend connection. |

::: tip
Duration values use human-readable format: `"5s"`, `"30s"`, `"2m"`, `"1h"`.
:::

## How it works

1. The proxy reads the client's handshake and login start packets.
2. It fires a `PreLoginEvent`, giving plugins a chance to deny the connection.
3. It fires a `PostLoginEvent`. No RSA key exchange, no encryption, no Mojang session verification.
4. It connects to the backend and forwards the raw handshake and login packets as received from the client.
5. The backend handles the login completion (sends `LoginSuccess` to the client).
6. The proxy enters the session loop, parsing and relaying packets in both directions.

The proxy does not send `LoginSuccess` to the client during its own auth phase. The backend is responsible for completing the login sequence. If the player ends up in limbo instead (because limbo handlers are configured or a plugin redirects there), the proxy sends `LoginSuccess` itself before entering limbo.

### Connection lifecycle

```
Client ──TCP──▶ Infrarust ──TCP──▶ Backend
                   │
         reads handshake + login start
                   │
         PreLoginEvent (plugins can deny)
         PostLoginEvent
                   │
         forwards raw handshake + login
         to backend (no auth, no encryption)
                   │
         backend sends LoginSuccess to client
                   │
         session loop ─────────▶ parsed packets flow
                                 both directions
```

### UUID generation

Since there's no Mojang session, the player's UUID comes from the client's login start packet. If the client doesn't provide one, the proxy generates a random v4 UUID. This means the same username can have different UUIDs across connections. If you need stable UUIDs for cracked players, handle that on the backend or through a plugin.

## Differences from client-only

| | Client-only | Offline |
|---|---|---|
| Mojang authentication | Yes | No |
| Client-side encryption | Yes (AES/CFB8) | No |
| `LoginSuccess` sent by proxy | Yes | No (backend sends it) |
| Packet inspection | Yes | Yes |
| Server switching | Yes | Yes |
| Limbo handlers | Yes | Yes |
| Cracked clients | No | Yes |

Both modes are intercepted: the proxy parses every packet, sessions are active, and plugins can inject packets or move players between servers.

## Server networks

Offline mode supports networks, just like client-only. Servers in the same network can switch players between each other.

```toml
name = "hub"
network = "main"
domains = ["play.mc.example.com"]
addresses = ["192.168.1.10:25565"]
proxy_mode = "offline"
```

```toml
name = "survival"
network = "main"
addresses = ["192.168.1.11:25565"]
proxy_mode = "offline"
```

You can mix offline and client-only servers within the same network. The `survival` server above has no `domains`, so players reach it only through a server switch from within the network.

::: info
Only intercepted modes (`client_only`, `offline`) can belong to a network. Forwarding modes (`passthrough`, `zero_copy`, `server_only`) are rejected during config validation if they specify a network.
:::

## Security considerations

::: danger No identity verification
Without Mojang authentication, anyone can connect with any username. A malicious player can impersonate an admin or any other player by using their username. Do not use offline mode on public-facing servers without additional authentication.
:::

If your server is public and you need cracked client support, put an authentication plugin on the backend (AuthMe, nLogin, etc.) that verifies players through a password or other mechanism before granting them access.

If you only need offline mode for local development, bind the proxy to `127.0.0.1` in the global config so it's not reachable from the network.

No traffic between the client and proxy is encrypted in offline mode. On a local network or behind a VPN this is fine. Over the public internet, anyone between the client and proxy can read the traffic. For public servers that need encryption, use [client-only mode](./client-only.md) instead.

## Compared to other modes

If offline mode doesn't fit your needs:

- Need authentication? Use [client-only](./client-only.md). Same packet inspection and server switching, with Mojang identity verification.
- Don't need server switching or plugins? Use [passthrough](./passthrough.md). Lower overhead, works with every Minecraft version.
- Need lower CPU on Linux without server switching? Use [zero-copy](./zerocopy.md).
