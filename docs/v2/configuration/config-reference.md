---
title: Configuration Reference
description: Complete reference for all infrarust.toml and server TOML options with types, defaults, and examples.
outline: [2, 3]
---

# Configuration Reference

Infrarust uses two kinds of TOML files:

- `infrarust.toml` — global proxy settings (bind address, rate limits, Docker, telemetry, etc.)
- `servers/*.toml` — one file per backend server (domains, addresses, proxy mode, MOTD, etc.)

All duration values use human-readable strings: `"5s"`, `"10m"`, `"1h30m"`.

## Global proxy config (`infrarust.toml`)

### Top-level options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `bind` | string | `"0.0.0.0:25565"` | Listen address and port |
| `max_connections` | integer | `0` | Maximum simultaneous connections. 0 = unlimited |
| `connect_timeout` | duration | `"5s"` | Timeout when connecting to a backend server |
| `receive_proxy_protocol` | boolean | `false` | Accept HAProxy v1/v2 PROXY protocol from upstream |
| `servers_dir` | string | `"./servers"` | Path to the directory containing server TOML files |
| `worker_threads` | integer | `0` | Number of tokio worker threads. 0 = auto (one per CPU core) |
| `so_reuseport` | boolean | `false` | Enable `SO_REUSEPORT` socket option (Linux only) |
| `unknown_domain_behavior` | string | `"default_motd"` | What to do when a player connects with an unknown domain. `"default_motd"` shows the default MOTD, `"drop"` silently closes the connection |

Minimal example:

```toml
bind = "0.0.0.0:25565"
servers_dir = "./servers"
```

### `[rate_limit]`

Per-IP rate limiting applied globally. Login attempts and status pings have separate limits.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `max_connections` | integer | `3` | Maximum login connections per IP per window |
| `window` | duration | `"10s"` | Time window for login rate limiting |
| `status_max` | integer | `30` | Maximum status ping connections per IP per window |
| `status_window` | duration | `"10s"` | Time window for status ping rate limiting |

```toml
[rate_limit]
max_connections = 5
window = "10s"
status_max = 50
status_window = "10s"
```

### `[status_cache]`

Caches status ping responses so the proxy doesn't forward every ping to the backend.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `ttl` | duration | `"5s"` | How long a cached status response stays valid |
| `max_entries` | integer | `1000` | Maximum number of cached entries |

```toml
[status_cache]
ttl = "5s"
max_entries = 1000
```

### `[keepalive]`

TCP keepalive probes to detect dead connections.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `time` | duration | `"30s"` | Idle time before the first probe |
| `interval` | duration | `"10s"` | Interval between probes |
| `retries` | integer | `3` | Number of failed probes before dropping the connection |

```toml
[keepalive]
time = "30s"
interval = "10s"
retries = 3
```

### `[ban]`

Persistent ban system with automatic expiration.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `file` | string | `"bans.json"` | Path to the JSON file storing active bans |
| `purge_interval` | duration | `"300s"` | How often expired bans are purged from the file |
| `enable_audit_log` | boolean | `true` | Log ban/unban operations |

```toml
[ban]
file = "./bans.json"
purge_interval = "300s"
enable_audit_log = true
```

### `[default_motd]`

MOTD shown when a player pings a domain that doesn't match any server. Uses the same `[motd]` format described in the server config section below.

```toml
[default_motd.offline]
text = "§cNo server found for this domain"
version_name = "Infrarust"
max_players = 0
```

### `[docker]`

Auto-discovers servers from Docker container labels. Requires Infrarust to be compiled with the `docker` feature.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `endpoint` | string | `"unix:///var/run/docker.sock"` | Docker daemon socket or TCP endpoint |
| `network` | string | none | Preferred Docker network name for resolving container addresses |
| `poll_interval` | duration | `"30s"` | Fallback polling interval for container changes |
| `reconnect_delay` | duration | `"5s"` | Delay before reconnecting after Docker daemon disconnection |

```toml
[docker]
endpoint = "unix:///var/run/docker.sock"
network = "my-minecraft-network"
poll_interval = "30s"
reconnect_delay = "5s"
```

### `[telemetry]`

OpenTelemetry export for metrics and traces. Omit this entire section to disable telemetry.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `false` | Master switch for telemetry export |
| `endpoint` | string | none | OTLP endpoint (e.g., `"http://localhost:4317"`) |
| `protocol` | string | `"grpc"` | Export protocol: `"grpc"` or `"http"` |

#### `[telemetry.metrics]`

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `true` | Enable metrics export |
| `export_interval` | duration | `"15s"` | How often metrics are pushed to the collector |

#### `[telemetry.traces]`

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `true` | Enable traces export |
| `sampling_ratio` | float | `0.1` | Sampling ratio for status pings (0.0 to 1.0). Login connections are always traced at 100% |

#### `[telemetry.resource]`

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `service_name` | string | `"infrarust"` | OpenTelemetry service name |
| `service_version` | string | crate version | OpenTelemetry service version |

```toml
[telemetry]
enabled = true
endpoint = "http://localhost:4317"
protocol = "grpc"

[telemetry.metrics]
enabled = true
export_interval = "15s"

[telemetry.traces]
enabled = true
sampling_ratio = 0.1

[telemetry.resource]
service_name = "infrarust"
```

### `[plugins.<id>]`

Plugin configuration, keyed by plugin ID.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `path` | string | none | Path to the plugin binary or library |
| `permissions` | array of strings | `[]` | Permissions granted to this plugin |
| `enabled` | boolean | none | Whether the plugin is enabled |

```toml
[plugins.auth]
path = "./plugins/libauth.so"
permissions = ["limbo", "command"]
enabled = true
```

---

## Server config (`servers/*.toml`)

Each file in the `servers_dir` directory defines one backend server. The filename (minus `.toml`) becomes the server's `id` unless overridden by `name`.

### Top-level options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `id` | string | from filename | Server identifier. Overridden by `name` if set |
| `name` | string | none | Human-readable name. Becomes the server ID if set. Must match `[a-z0-9_-]+` |
| `network` | string | none | Network group for server switching. Only servers in the same network can switch between each other. Omit to isolate the server |
| `domains` | array of strings | `[]` | Domains that route to this server. Supports wildcards like `"*.mc.example.com"`. Empty means the server is only reachable via server switching |
| `addresses` | array of strings | **required** | Backend server addresses in `"host:port"` format. Port defaults to 25565 if omitted |
| `proxy_mode` | string | `"passthrough"` | How the proxy handles traffic. See [Proxy modes](#proxy-modes) |
| `send_proxy_protocol` | boolean | `false` | Send PROXY protocol header to the backend |
| `domain_rewrite` | string or table | `"none"` | Rewrite the domain in the handshake before forwarding. `"none"`, `"from_backend"`, or `{ explicit = "domain" }` |
| `max_players` | integer | `0` | Maximum players on this server. 0 = unlimited |
| `disconnect_message` | string | `"Server is currently unreachable. Please try again later."` | Message sent to the player when the backend is unreachable |
| `limbo_handlers` | array of strings | `[]` | Plugin IDs for limbo handler chain, executed in order |

Minimal example:

```toml
domains = ["mc.example.com"]
addresses = ["127.0.0.1:25565"]
```

Full example:

```toml
name = "survival"
network = "main"
domains = ["play.example.com", "*.mc.example.com"]
addresses = ["10.0.0.5:25565"]
proxy_mode = "client_only"
send_proxy_protocol = false
domain_rewrite = "none"
max_players = 100
disconnect_message = "§cServer is offline. Try again later."
limbo_handlers = ["server_wake"]
```

### Proxy modes

| Mode | TOML value | Description |
|------|------------|-------------|
| Passthrough | `"passthrough"` | Raw forwarding via `tokio::io::copy_bidirectional`. Default mode |
| Zero Copy | `"zero_copy"` | Raw forwarding via `splice(2)` syscall. Linux only |
| Client Only | `"client_only"` | Proxy handles Mojang authentication. Backend runs in `online_mode=false` |
| Offline | `"offline"` | No authentication. Transparent relay with packet parsing |
| Server Only | `"server_only"` | Authentication handled entirely by the backend |
| Full | `"full"` | Encryption on both proxy-to-client and proxy-to-backend sides |

::: tip
`passthrough`, `zero_copy`, and `server_only` forward raw bytes after the handshake. The proxy cannot inspect or modify packets in these modes.

`client_only`, `offline`, and `full` parse packets, which enables features like server switching and limbo handlers.
:::

### `[motd]`

MOTD entries for each server state. Each sub-table is optional.

Available states: `online`, `offline`, `sleeping`, `starting`, `crashed`, `stopping`, `unreachable`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `text` | string | **required** | MOTD text. Supports Minecraft `§` formatting codes |
| `favicon` | string | none | Path to a PNG file, a base64-encoded PNG, or a URL |
| `version_name` | string | none | Version string shown in the client server list |
| `max_players` | integer | none | Max player count shown in the server list |

```toml
[motd.online]
text = "§aServer Online §7— Welcome"
favicon = "./icon.png"

[motd.offline]
text = "§cServer Offline"
version_name = "Maintenance"
max_players = 0

[motd.sleeping]
text = "§7Server sleeping — connect to wake it up"
version_name = "Sleeping"
```

### `[timeouts]`

Server-specific timeout overrides. If omitted, the global `connect_timeout` applies for the connect phase.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `connect` | duration | `"5s"` | Backend connection timeout |
| `read` | duration | `"30s"` | Read timeout on the backend socket |
| `write` | duration | `"30s"` | Write timeout on the backend socket |

```toml
[timeouts]
connect = "10s"
read = "60s"
write = "60s"
```

### `[ip_filter]`

IP-based access control using CIDR notation. If `whitelist` is set, only matching IPs can connect. Otherwise, `blacklist` rejects matching IPs.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `whitelist` | array of strings | `[]` | CIDR ranges to allow. Takes priority over blacklist |
| `blacklist` | array of strings | `[]` | CIDR ranges to reject |

```toml
[ip_filter]
whitelist = ["192.168.1.0/24", "10.0.0.0/8"]
```

```toml
[ip_filter]
blacklist = ["203.0.113.0/24"]
```

::: warning
If both `whitelist` and `blacklist` are set, only the whitelist is evaluated. The blacklist is ignored when a whitelist is present.
:::

### `[server_manager]`

Automatic server start/stop management. The `type` field selects the provider.

#### Local process (`type = "local"`)

Launches a local process (typically `java` or `docker`).

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `type` | string | | Must be `"local"` |
| `command` | string | **required** | Command to execute |
| `working_dir` | string | **required** | Working directory for the process |
| `args` | array of strings | `[]` | Command arguments |
| `ready_pattern` | string | `'For help, type "help"'` | Pattern in stdout that signals the server is ready |
| `shutdown_timeout` | duration | `"30s"` | Time to wait for graceful shutdown |
| `shutdown_after` | duration | none | Shut down the server after this idle duration. Omit to disable |
| `start_timeout` | duration | `"60s"` | Maximum time to wait for the server to become ready |

```toml
[server_manager]
type = "local"
command = "java"
args = ["-Xmx4G", "-jar", "server.jar", "nogui"]
working_dir = "/opt/minecraft/survival"
ready_pattern = "For help, type"
shutdown_after = "10m"
start_timeout = "120s"
```

#### Pterodactyl (`type = "pterodactyl"`)

Controls a server via the Pterodactyl panel API.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `type` | string | | Must be `"pterodactyl"` |
| `api_url` | string | **required** | Panel API URL |
| `api_key` | string | **required** | API key with server control permissions |
| `server_id` | string | **required** | Pterodactyl server identifier |
| `shutdown_after` | duration | none | Shut down after this idle duration. Omit to disable |
| `start_timeout` | duration | `"60s"` | Maximum time to wait for the server to start |
| `poll_interval` | duration | `"5s"` | How often to poll the API for server state |

```toml
[server_manager]
type = "pterodactyl"
api_url = "https://panel.example.com"
api_key = "ptlc_xxxxxxxxxxxxx"
server_id = "abc12345"
shutdown_after = "15m"
```

#### Crafty Controller (`type = "crafty"`)

Controls a server via the Crafty Controller API.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `type` | string | | Must be `"crafty"` |
| `api_url` | string | **required** | Crafty API URL |
| `api_key` | string | **required** | API key |
| `server_id` | string | **required** | Crafty server identifier |
| `shutdown_after` | duration | none | Shut down after this idle duration. Omit to disable |
| `start_timeout` | duration | `"60s"` | Maximum time to wait for the server to start |
| `poll_interval` | duration | `"5s"` | How often to poll the API for server state |

```toml
[server_manager]
type = "crafty"
api_url = "https://crafty.example.com:8443"
api_key = "your-crafty-api-key"
server_id = "12345678-abcd-1234-abcd-123456789abc"
shutdown_after = "15m"
```

---

## Full example

A complete `infrarust.toml` with all sections:

```toml
bind = "0.0.0.0:25565"
servers_dir = "./servers"
max_connections = 500
connect_timeout = "5s"
worker_threads = 0
receive_proxy_protocol = false
so_reuseport = false
unknown_domain_behavior = "default_motd"

[rate_limit]
max_connections = 3
window = "10s"
status_max = 30
status_window = "10s"

[status_cache]
ttl = "5s"
max_entries = 1000

[keepalive]
time = "30s"
interval = "10s"
retries = 3

[ban]
file = "bans.json"
purge_interval = "300s"
enable_audit_log = true

[default_motd.offline]
text = "§cNo server found for this domain"
version_name = "Infrarust"
max_players = 0

[docker]
endpoint = "unix:///var/run/docker.sock"
network = "minecraft-net"
poll_interval = "30s"
reconnect_delay = "5s"

[telemetry]
enabled = true
endpoint = "http://localhost:4317"
protocol = "grpc"

[telemetry.metrics]
enabled = true
export_interval = "15s"

[telemetry.traces]
enabled = true
sampling_ratio = 0.1

[telemetry.resource]
service_name = "infrarust"

[plugins.auth]
path = "./plugins/libauth.so"
permissions = ["limbo", "command"]
enabled = true
```

A server file (`servers/survival.toml`) using most options:

```toml
name = "survival"
network = "main"
domains = ["play.example.com"]
addresses = ["10.0.0.5:25565"]
proxy_mode = "client_only"
max_players = 100
disconnect_message = "§cSurvival is offline. Try again later."
limbo_handlers = ["server_wake"]

[motd.online]
text = "§aSurvival §7— Online"
favicon = "./icons/survival.png"

[motd.sleeping]
text = "§7Survival §8— Sleeping, connect to wake"
version_name = "Sleeping"

[timeouts]
connect = "10s"
read = "60s"
write = "60s"

[ip_filter]
blacklist = ["203.0.113.0/24"]

[server_manager]
type = "local"
command = "java"
args = ["-Xmx4G", "-jar", "server.jar", "nogui"]
working_dir = "/opt/minecraft/survival"
shutdown_after = "10m"
start_timeout = "120s"
```
