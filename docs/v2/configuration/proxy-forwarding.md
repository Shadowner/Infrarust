---
title: Proxy Forwarding
description: Forward player identity (UUID, skin, IP) to backend servers using Velocity, BungeeCord, or BungeeGuard protocols.
outline: [2, 3]
---

# Proxy Forwarding

When the proxy handles authentication (in `client_only` or `offline` mode), the backend runs in offline mode and doesn't know who the player is. Proxy forwarding transmits the player's real identity to the backend through a side channel: UUID, username, skin textures, and IP address.

Without forwarding, every player appears as an offline-mode player. Skins break, UUIDs are wrong, and IP-based features stop working.

## Forwarding modes

Infrarust supports four forwarding modes:

| Mode | Protocol | Security | Backend support |
|---|---|---|---|
| `none` | — | — | Backend handles auth itself |
| `velocity` | Plugin message channel | HMAC-SHA256 signed | Paper, Purpur, Fabric (via mod) |
| `bungeecord` | Handshake injection | None (trust-based) | Spigot, Paper, most forks |
| `bungeeguard` | Handshake injection + token | Shared token | Spigot/Paper with BungeeGuard plugin |

## Configuration

Forwarding is configured globally in `infrarust.toml` under the `[forwarding]` section:

::: code-group

```toml [Velocity (recommended)]
[forwarding]
mode = "velocity"
secret_file = "forwarding.secret"
```

```toml [BungeeCord]
[forwarding]
mode = "bungeecord"
```

```toml [BungeeGuard]
[forwarding]
mode = "bungeeguard"
secret_file = "forwarding.secret"
```

```toml [Disabled (default)]
[forwarding]
mode = "none"
```

:::

### Config options

| Option | Type | Default | Description |
|---|---|---|---|
| `mode` | string | `"none"` | Forwarding mode: `none`, `velocity` (alias: `modern`), `bungeecord` (alias: `legacy`), `bungeeguard`. |
| `secret_file` | path | `"forwarding.secret"` | Path to the shared secret file. Used by `velocity` and `bungeeguard` modes. Created automatically if it doesn't exist. |
| `bungeecord_channel` | bool | `true` | Enable BungeeCord plugin messaging channel support. |

### Per-server override

Individual servers can override the global forwarding mode:

```toml
domains = ["lobby.mc.example.com"]
addresses = ["10.0.1.10:25565"]
proxy_mode = "client_only"
forwarding_mode = "velocity"
```

Useful for mixed networks, like a Paper lobby with Velocity forwarding alongside a vanilla server with forwarding disabled.

## Velocity forwarding

Velocity is the recommended forwarding mode. It uses a plugin message channel (`velocity:player_info`) during login, and every message is signed with HMAC-SHA256 to prevent forgery.

### How it works

1. The proxy connects to the backend and starts the login sequence.
2. The backend sends a `LoginPluginRequest` on the `velocity:player_info` channel, advertising its supported forwarding version.
3. The proxy negotiates the highest common version, builds a payload with the player's identity, signs it with HMAC-SHA256, and sends it back as a `LoginPluginResponse`.
4. The backend verifies the signature using the same shared secret and accepts the player with the forwarded identity.

### Secret file

The secret file is shared between the proxy and all backend servers. If the file doesn't exist, Infrarust generates a random 12-character alphanumeric secret and creates it with `0600` permissions (Unix).

Copy this secret to each backend:

- Paper / Purpur: `config/paper-global.yml` under `proxies.velocity.secret`
- Fabric (FabricProxy-Lite): `config/FabricProxy-Lite.toml`

::: warning
The secret must be identical on the proxy and every backend. If they differ, players get rejected with a signature verification failure.
:::

### Version negotiation

Infrarust supports Velocity forwarding versions 1 through 4. The proxy and backend negotiate the version automatically based on the backend's capabilities and the client's protocol version:

| Version | Minecraft | Content |
|---|---|---|
| 1 | 1.13+ | IP, UUID, username, skin properties |
| 2 | 1.19+ | Adds chat session key (if present) |
| 3 | 1.19.1+ | Adds holder UUID for chat session |
| 4 | 1.19.3+ | Latest format |

No manual configuration needed.

### Backend configuration (Paper)

In your Paper server's `config/paper-global.yml`:

```yaml
proxies:
  velocity:
    enabled: true
    online-mode: true
    secret: "paste-your-secret-here"
```

And in `server.properties`:

```properties
online-mode=false
```

## BungeeCord forwarding

Legacy BungeeCord forwarding injects player data into the handshake packet's server address field using null-byte separators.

### How it works

The proxy rewrites the handshake's `server_address` field from:

```
play.example.com
```

to:

```
play.example.com\0192.168.1.42\0069a79f444e94726a5befca90e38aaf5\0[{"name":"textures","value":"...","signature":"..."}]
```

The four segments are: original domain, player's real IP, UUID (without dashes), and a JSON array of profile properties (skin textures).

::: danger Insecure by design
BungeeCord forwarding has no authentication. Anyone who can reach your backend directly can forge a handshake and impersonate any player. Only use this if your backends sit on a private network with no direct access.

If backend ports are exposed, use `velocity` or `bungeeguard` instead.
:::

### Backend configuration

In your Spigot/Paper server's `spigot.yml`:

```yaml
settings:
  bungeecord: true
```

And in `server.properties`:

```properties
online-mode=false
```

## BungeeGuard forwarding

BungeeGuard extends legacy BungeeCord forwarding with a shared token. It uses the same handshake injection format but adds a `bungeeguard-token` property to the profile properties array.

### How it works

Same as BungeeCord forwarding, but the proxy appends an extra property:

```json
{"name": "bungeeguard-token", "value": "your-secret-token", "signature": ""}
```

The backend's BungeeGuard plugin checks this token against its allowlist and rejects connections with missing or invalid tokens.

### Backend configuration

1. Install the [BungeeGuard](https://github.com/lucko/BungeeGuard) plugin on your backend server.
2. Add the token from your `forwarding.secret` file to BungeeGuard's `config.yml`:

```yaml
allowed-tokens:
  - "paste-your-secret-here"
```

3. Set `bungeecord: true` in `spigot.yml` and `online-mode=false` in `server.properties`.

## BungeeCord channel permissions

When `bungeecord_channel` is enabled (default), the proxy handles BungeeCord plugin messaging channels. You can control which operations backends are allowed to perform through the channel:

```toml
[forwarding]
mode = "velocity"
bungeecord_channel = true

[forwarding.channel_permissions]
connect = true
connect_other = false
ip = true
ip_other = true
player_count = true
player_list = true
get_servers = true
get_server = true
get_player_server = true
forward = true
forward_to_player = true
uuid = true
uuid_other = true
server_ip = true
message = false
message_raw = false
kick_player = false
kick_player_raw = false
```

Permissions that default to `false` are write operations (sending messages, kicking players, connecting other players). Read operations default to `true`.

## Choosing a mode

Pick Velocity unless you have a reason not to. It's the only mode with cryptographic signing, and Paper, Purpur, and Fabric all support it.

If your backends can't do Velocity forwarding, BungeeGuard adds token-based authentication on top of the legacy format. Better than nothing.

Plain BungeeCord forwarding is only appropriate for fully isolated networks where nobody can reach backend ports directly. If ports are exposed, anyone can impersonate any player.

Set the mode to `none` when the backend handles its own auth, which is the case for `passthrough` and `server_only` proxy modes where the proxy doesn't touch the login sequence.
