---
title: Client-Only Mode
description: Proxy handles Mojang authentication while the backend runs in offline mode, enabling packet inspection and server switching.
outline: [2, 3]
---

# Client-Only Mode

In client-only mode, the proxy terminates the client's connection and handles Mojang authentication itself. The backend server must run with `online-mode=false` because the proxy, not the backend, verifies the player's identity.

This is the mode you need for server networks, packet inspection, and plugin features that interact with players.

## When to use it

Use client-only when you need any of these:

- Server switching within a network (moving players between backends without reconnecting)
- Packet inspection or modification by plugins
- Centralized authentication across multiple backends
- Features like limbo handlers, codec filters, or event-driven packet injection

## Configuration

A minimal server config file:

```toml
domains = ["mc.example.com"]
addresses = ["192.168.1.10:25565"]
proxy_mode = "client_only"
```

With Docker labels:

```yaml
labels:
  infrarust.domains: "mc.example.com"
  infrarust.proxy_mode: "client_only"
```

::: danger Backend must be offline
Your backend Minecraft server **must** have `online-mode=false` in its `server.properties`. The proxy already verified the player's identity, so the backend doesn't need to check again. If the backend has `online-mode=true`, players will fail to connect because the backend will try to re-authenticate them.
:::

### Full example

This uses every option that applies to client-only mode:

```toml
name = "survival"
network = "main"
domains = ["survival.mc.example.com", "*.survival.example.com"]
addresses = ["10.0.1.10:25565", "10.0.1.11:25565"]
proxy_mode = "client_only"
send_proxy_protocol = false
max_players = 100
disconnect_message = "Survival server is offline. Try again later."
limbo_handlers = ["motd", "tablist"]

[timeouts]
connect = "3s"
read = "30s"
write = "30s"

[motd.online]
text = "§6Survival — Online"
```

### Config options

| Option | Type | Default | Description |
|---|---|---|---|
| `domains` | string array | `[]` | Domains that route to this server. Supports wildcards (`*.mc.example.com`). Can be empty if the server is only reachable via server switching within a network. |
| `addresses` | string array | required | Backend addresses in `host:port` format. |
| `proxy_mode` | string | `"passthrough"` | Set to `"client_only"`. |
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
3. It performs Mojang authentication: sends an `EncryptionRequest` with the proxy's RSA public key, reads the `EncryptionResponse`, decrypts the shared secret, and verifies the session against `sessionserver.mojang.com`.
4. On success, it sends `LoginSuccess` to the client with the player's UUID, username, and skin properties from Mojang.
5. For 1.20.2+, it waits for the client's `LoginAcknowledged` packet and transitions to the Configuration state. For older versions, it transitions directly to Play state.
6. It fires a `PostLoginEvent`.
7. It connects to the backend in offline mode, replaying the login sequence.
8. It enters the session loop, parsing and relaying packets in both directions.

Because the proxy parses every packet, sessions are marked as active. Plugins can inject packets into the stream, and the proxy can move the player to a different backend without dropping the client connection.

### Connection lifecycle

```
Client ──TCP──▶ Infrarust ──TCP──▶ Backend
                   │
         reads handshake + login start
                   │
         PreLoginEvent (plugins can deny)
                   │
         EncryptionRequest ──▶ Client
         Client ──▶ EncryptionResponse
                   │
         decrypt shared secret + verify token
         verify session with Mojang
                   │
         LoginSuccess ──▶ Client
         (1.20.2+: LoginAcknowledged)
                   │
         connects to backend (offline mode)
                   │
         session loop ─────────▶ parsed packets flow
                                 both directions
```

### Authentication details

The proxy generates an RSA-1024 key pair on startup and reuses it for all connections. During the login sequence, it:

1. Sends the DER-encoded public key to the client in an `EncryptionRequest`.
2. Decrypts the client's shared secret and verify token using RSA PKCS#1 v1.5.
3. Computes the Minecraft server hash (a non-standard SHA-1 with signed BigInt output).
4. Calls `https://sessionserver.mojang.com/session/minecraft/hasJoined` with the username and server hash.
5. Receives the player's game profile (UUID, username, skin textures) from Mojang.
6. Enables AES/CFB8 encryption on the client connection using the shared secret.

The backend never sees encrypted traffic. The proxy connects to the backend as an offline-mode client.

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

The `survival` server has no `domains` entry. Players can only reach it through a server switch from within the network.

::: info
Only intercepted modes (`client_only`, `offline`) can belong to a network. Forwarding modes (`passthrough`, `zero_copy`, `server_only`) are rejected during config validation if they specify a network.
:::

## Server manager

You can pair client-only mode with a server manager that starts and stops the backend automatically:

```toml
name = "creative"
domains = ["creative.mc.example.com"]
addresses = ["127.0.0.1:25566"]
proxy_mode = "client_only"

[server_manager]
type = "local"
command = "java"
working_dir = "/opt/minecraft/creative"
args = ["-Xmx4G", "-jar", "server.jar", "nogui"]
ready_pattern = 'For help, type "help"'
shutdown_timeout = "30s"
shutdown_after = "15m"
```

When a player connects and the backend is down, the proxy starts the server process and holds the player in a limbo state until the backend is ready.

## Constraints

Client-only is an intercepted mode. These rules apply:

- The backend **must** run with `online-mode=false`. The proxy already authenticated the player.
- Works with Minecraft versions that Infrarust can parse (currently 1.7 through 1.21.x). Future protocol versions require an Infrarust update.
- Domains are optional if the server belongs to a network and is only reachable via server switching.
- Higher resource usage than forwarding modes because the proxy parses every packet instead of copying raw bytes.

## Compared to other modes

If client-only doesn't fit your needs:

- Don't need server switching or plugins? Use [passthrough](./passthrough.md). Lower overhead, works with every Minecraft version.
- Need lower CPU on Linux without server switching? Use [zero-copy](./zerocopy.md).
- Need server switching without Mojang authentication? Use [offline](./offline.md). Same capabilities, but any username can connect.
