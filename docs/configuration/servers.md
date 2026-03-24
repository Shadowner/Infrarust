---
title: Server Definitions
description: How to define backend servers in Infrarust, with all available options for domains, addresses, proxy modes, timeouts, MOTDs, and server management.
---

# Server Definitions

Each `.toml` file in your `servers/` directory defines one backend Minecraft server. The filename (without the `.toml` extension) becomes the server's ID unless you set `id` or `name` explicitly.

## Minimal example

A server only needs an address:

```toml
domains = ["survival.example.com"]
addresses = ["127.0.0.1:25565"]
```

Without `domains`, the server won't receive traffic from domain routing but can still be reached through server switching within a network.

## Full example

```toml
name = "survival"
network = "main"
domains = ["survival.mc.example.com", "*.survival.example.com"]
addresses = ["10.0.1.10:25565", "10.0.1.11:25565"]
proxy_mode = "passthrough"
send_proxy_protocol = false
domain_rewrite = "none"
max_players = 100
disconnect_message = "Survival is down for maintenance."
limbo_handlers = ["auth", "antibot"]

[timeouts]
connect = "3s"
read = "30s"
write = "30s"

[ip_filter]
whitelist = ["192.168.1.0/24"]
blacklist = ["10.0.0.5/32"]

[motd.online]
text = "§aSurvival §7— §fWelcome!"
favicon = "./icons/survival.png"
version_name = "Survival 1.21"
max_players = 100

[motd.sleeping]
text = "§eSurvival §7— §fConnect to wake up!"
version_name = "Server Sleeping"

[motd.starting]
text = "§eSurvival §7— §fStarting..."

[server_manager]
type = "pterodactyl"
api_url = "https://panel.example.com"
api_key = "ptlc_xxxxx"
server_id = "abc123"
shutdown_after = "10m"
start_timeout = "60s"
poll_interval = "5s"
```

## Fields reference

### Identity

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `id` | string | filename | Unique identifier. Set automatically from the filename if omitted. |
| `name` | string | — | Human-readable name. Takes priority over `id` as the server's identity. Must match `[a-z0-9_-]+`, max 64 characters. |
| `network` | string | — | Network group for server switching. Players can only switch between servers in the same network. Omit to isolate the server. Must match `[a-z0-9_-]+`. |

The effective server ID is resolved as: `name` > `id` > `"unknown"`. Duplicate IDs across all server files cause a startup error.

### Routing

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `domains` | list of strings | `[]` | Domains that route players to this server. Supports wildcards like `*.mc.example.com`. |
| `addresses` | list of strings | *required* | Backend server addresses in `host:port` format. If you omit the port, it defaults to `25565`. |

`addresses` is the only required field. You must provide at least one address.

Wildcard domains match any subdomain at that level. `*.mc.example.com` matches `survival.mc.example.com` and `creative.mc.example.com`, but not `mc.example.com` itself. Exact matches always take priority over wildcards.

::: warning
Forwarding proxy modes (`passthrough`, `zero_copy`, `server_only`) require at least one domain. These modes cannot belong to a network because they don't support server switching.
:::

### Proxy behavior

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `proxy_mode` | string | `"passthrough"` | How the proxy handles traffic. See below. |
| `send_proxy_protocol` | bool | `false` | Send HAProxy proxy protocol v1/v2 headers to the backend. |
| `domain_rewrite` | string | `"none"` | Rewrite the domain in the Minecraft handshake before forwarding. |
| `max_players` | integer | `0` | Maximum players allowed on this server. `0` means unlimited. |
| `disconnect_message` | string | `"Server is currently unreachable. Please try again later."` | Message shown to players when the backend is unreachable. |
| `limbo_handlers` | list of strings | `[]` | Plugin IDs for the limbo handler chain, executed in order. |

#### Proxy modes

All modes use `snake_case` in the config file.

| Mode | Config value | Description |
|------|-------------|-------------|
| Passthrough | `"passthrough"` | Raw TCP forwarding. No packet inspection. Default. |
| Zero-copy | `"zero_copy"` | Raw forwarding using `splice(2)`. Linux only. |
| Client-only | `"client_only"` | Proxy handles Mojang authentication. Backend runs in `online_mode=false`. |
| Offline | `"offline"` | No authentication. Transparent relay. |
| Server-only | `"server_only"` | Authentication handled entirely by the backend. |
| Full | `"full"` | Encryption on both client and server sides. |

Passthrough, zero-copy, and server-only are "forwarding" modes: the proxy relays raw bytes after the handshake. Client-only, offline, and full are "intercepted" modes: the proxy parses and may modify packets.

::: tip
Zero-copy mode only works on Linux. On other platforms, Infrarust logs a warning and still accepts the config, but performance won't differ from passthrough.
:::

#### Domain rewrite

Controls what domain the backend sees in the Minecraft handshake packet.

| Value | Description |
|-------|-------------|
| `"none"` | Forward the original domain as-is. Default. |
| `"from_backend"` | Use the host from the first entry in `addresses`. |
| `{ "explicit": "mc.local" }` | Rewrite to the specified domain. |

The explicit variant uses TOML inline table syntax:

```toml
domain_rewrite = { "explicit" = "mc.local" }
```

### Timeouts

Override global timeout values for this server. Omit the entire `[timeouts]` section to use the global defaults from `infrarust.toml`.

```toml
[timeouts]
connect = "3s"
read = "30s"
write = "30s"
```

| Field | Default | Description |
|-------|---------|-------------|
| `connect` | `"5s"` | Timeout for establishing a connection to the backend. |
| `read` | `"30s"` | Timeout for reading data from the backend. |
| `write` | `"30s"` | Timeout for writing data to the backend. |

All durations use human-readable strings: `"5s"`, `"10m"`, `"1h30m"`.

### IP filtering

Restrict which IP addresses can connect to this server. Addresses use CIDR notation.

```toml
[ip_filter]
whitelist = ["192.168.1.0/24", "10.0.0.0/8"]
blacklist = ["10.0.0.5/32"]
```

If `whitelist` is non-empty, only IPs matching the whitelist are allowed. The whitelist is evaluated before the blacklist.

### MOTD

Configure the server list appearance for different server states. Each state is optional.

```toml
[motd.online]
text = "§aSurvival §7— §fWelcome!"
favicon = "./icons/survival.png"
version_name = "Survival 1.21"
max_players = 100

[motd.offline]
text = "§cSurvival §7— §fOffline"

[motd.sleeping]
text = "§eSurvival §7— §fConnect to wake up!"
version_name = "Server Sleeping"

[motd.starting]
text = "§eSurvival §7— §fStarting..."

[motd.crashed]
text = "§4Survival §7— §fCrashed"

[motd.stopping]
text = "§6Survival §7— §fStopping..."

[motd.unreachable]
text = "§cSurvival §7— §fUnreachable"
```

Available states: `online`, `offline`, `sleeping`, `starting`, `crashed`, `stopping`, `unreachable`.

Each MOTD entry supports these fields:

| Field | Type | Description |
|-------|------|-------------|
| `text` | string | MOTD text. Supports Minecraft formatting codes (`§a`, `§b`, etc.). |
| `favicon` | string | Path to a 64x64 PNG file, a base64-encoded image, or a URL. |
| `version_name` | string | Version text shown in the client server list. |
| `max_players` | integer | Max player count displayed in the server list. |

### Server manager

Infrarust can start and stop backend servers automatically. When all players disconnect, the server shuts down after the configured idle timeout. When a new player connects, the server starts again.

Three manager types are available: `local`, `pterodactyl`, and `crafty`.

#### Local process

Starts the Minecraft server as a child process.

```toml
[server_manager]
type = "local"
command = "java"
working_dir = "/opt/minecraft/creative"
args = ["-Xmx4G", "-jar", "server.jar", "nogui"]
ready_pattern = 'For help, type "help"'
shutdown_timeout = "30s"
shutdown_after = "15m"
start_timeout = "60s"
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `command` | string | *required* | Executable to run. |
| `working_dir` | string | *required* | Working directory for the process. |
| `args` | list of strings | `[]` | Arguments passed to the command. |
| `ready_pattern` | string | `'For help, type "help"'` | Log line that indicates the server is ready. |
| `shutdown_timeout` | duration | `"30s"` | How long to wait for graceful shutdown before killing the process. |
| `shutdown_after` | duration | — | Idle time before auto-shutdown. Omit to keep the server running. |
| `start_timeout` | duration | `"60s"` | How long to wait for the ready pattern before giving up. |

#### Pterodactyl

Manages a server through the Pterodactyl panel API.

```toml
[server_manager]
type = "pterodactyl"
api_url = "https://panel.example.com"
api_key = "ptlc_xxxxx"
server_id = "abc123"
shutdown_after = "10m"
start_timeout = "60s"
poll_interval = "5s"
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `api_url` | string | *required* | Pterodactyl panel URL. |
| `api_key` | string | *required* | API key for authentication. |
| `server_id` | string | *required* | Server identifier in the panel. |
| `shutdown_after` | duration | — | Idle time before auto-shutdown. |
| `start_timeout` | duration | `"60s"` | How long to wait for the server to start. |
| `poll_interval` | duration | `"5s"` | How often to check the server's state. |

#### Crafty Controller

Manages a server through the Crafty Controller API.

```toml
[server_manager]
type = "crafty"
api_url = "https://crafty.example.com"
api_key = "your-api-key"
server_id = "server-uuid"
shutdown_after = "10m"
start_timeout = "60s"
poll_interval = "5s"
```

The fields are identical to the Pterodactyl manager.

## Validation rules

Infrarust validates every server file at startup and on hot-reload. Invalid files are rejected with a clear error message.

- `addresses` must contain at least one entry.
- Forwarding modes (`passthrough`, `zero_copy`, `server_only`) require at least one domain.
- Forwarding modes cannot set `network` (they don't support server switching).
- `name` and `network` must match `[a-z0-9_-]+` and be at most 64 characters.
- Domain strings cannot be empty.
- No two server files can share the same effective ID.

All config files use strict parsing. Unknown fields cause a parse error rather than being silently ignored.
