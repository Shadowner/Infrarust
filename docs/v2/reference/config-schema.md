---
title: Configuration Reference
description: Complete reference for all infrarust.toml and server TOML options with types, defaults, and examples.
outline: [2, 3]
---

# Configuration Reference

Infrarust uses two kinds of TOML files:

- `infrarust.toml` holds the global proxy settings (bind address, rate limits, Docker, telemetry, and so on).
- `servers/*.toml` is one file per backend server (domains, addresses, proxy mode, MOTD, and so on).

All duration values use human-readable strings: `"5s"`, `"10m"`, `"1h30m"`.

## Global proxy config (`infrarust.toml`)

### Top-level options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `bind` | string | `"0.0.0.0:25565"` | Listen address and port |
| `max_connections` | integer | `0` | Maximum simultaneous connections. 0 = unlimited |
| `connect_timeout` | duration | `"5s"` | Timeout when connecting to a backend server |
| `connect_max_attempts` | integer | `3` | Backend addresses tried before giving up. 0 = try them all |
| `receive_proxy_protocol` | boolean | `false` | Accept HAProxy v1/v2 PROXY protocol from upstream |
| `servers_dir` | string | `"./servers"` | Path to the directory containing server TOML files |
| `plugins_dir` | string | `"./plugins"` | Path to the directory containing WASM plugin files |
| `worker_threads` | integer | `0` | Number of tokio worker threads. 0 = auto (one per CPU core) |
| `so_reuseport` | boolean | `false` | Enable `SO_REUSEPORT` socket option (Linux only) |
| `unknown_domain_behavior` | string | `"default_motd"` | What to do when a player connects with an unknown domain. `"default_motd"` shows the default MOTD, `"drop"` silently closes the connection |
| `announce_proxy_commands` | boolean | `true` | Advertise built-in `/ir` subcommands to clients that support command suggestions |

Parsing is strict. The global file deserializes with `deny_unknown_fields`, so an unknown or misspelled top-level key is a hard error rather than a silent no-op. The same applies to every table below.

Minimal example:

```toml
bind = "0.0.0.0:25565"
servers_dir = "./servers"
```

### `[rate_limit]`

Per-IP rate limiting. Login attempts and status pings have separate limits. Rate limiting is off by default; set `enabled = true` to turn it on.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `false` | Master switch for rate limiting |
| `max_connections` | integer | `3` | Maximum login connections per IP per window |
| `window` | duration | `"10s"` | Time window for login rate limiting |
| `status_max` | integer | `300` | Maximum status ping connections per IP per window |
| `status_window` | duration | `"10s"` | Time window for status ping rate limiting |

```toml
[rate_limit]
enabled = true
max_connections = 5
window = "10s"
status_max = 300
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

### `[ip_filter]`

CIDR-based access control can be set globally to gate every connection before routing, using the same fields and precedence as the per-server `[ip_filter]` documented in the server config section below. A global whitelist or blacklist applies to all incoming connections; a per-server filter narrows access further for one backend.

```toml
[ip_filter]
blacklist = ["203.0.113.0/24"]
```

### `[forwarding]`

Player info forwarding to the backend, the same mechanism Velocity and BungeeCord use to pass the real player identity and IP through the proxy. Set this globally to apply one mode to every server.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `mode` | string | `"none"` | Forwarding scheme: `"none"`, `"bungee_cord"`, `"bungee_guard"`, or `"velocity"` |
| `secret_file` | string | `"forwarding.secret"` | Path to the shared secret file for `velocity` and `bungee_guard`. Created automatically if absent |
| `bungeecord_channel` | boolean | `true` | Handle BungeeCord plugin messaging channel requests |

`mode` accepts two aliases for backward compatibility: `"legacy"` maps to `bungee_cord` and `"modern"` maps to `velocity`. A `[forwarding.channel_permissions]` sub-table controls which BungeeCord channel subchannels are answered (Connect, IP, PlayerCount, and similar); most are enabled by default while `ConnectOther`, `Message`, `MessageRaw`, `KickPlayer`, and `KickPlayerRaw` are off. See [Proxy forwarding](../configuration/proxy-forwarding) for the protocol details.

```toml
[forwarding]
mode = "velocity"
secret_file = "forwarding.secret"
```

### `[web]`

Web admin API and UI. Absent from the file means the web plugin is not loaded.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enable_api` | boolean | `true` | Serve the REST admin API |
| `enable_webui` | boolean | follows `enable_api` | Serve the bundled web UI. Cannot be `true` while `enable_api` is `false` |
| `bind` | string | `"127.0.0.1:8080"` | Listen address for the web server |
| `api_key` | string | none | API key for authenticating requests. Must be at least 16 characters |
| `cors_origins` | array of strings | `[]` | Allowed CORS origins |

`enable_webui` follows `enable_api` unless you set it, so `enable_api = false` alone turns the UI off too. Setting `enable_webui = true` with `enable_api = false` is a configuration error: the UI is served by the API's HTTP server and calls it for every screen. If `bind` is a loopback address and no `api_key` is set, Infrarust generates an ephemeral key at startup and logs a warning. If `bind` is reachable from outside the host and no key (or the placeholder `CHANGE-ME`) is set, Infrarust refuses to start. A key shorter than 16 characters is refused on any bind, and the API refuses to write a config that would fail either rule. A `[web.rate_limit]` sub-table sets `requests_per_minute` (default `60`).

```toml
[web]
enable_api = true
enable_webui = true
bind = "127.0.0.1:8080"
api_key = "a-strong-api-key-at-least-16-chars"
```

::: warning
Do not expose the web API on a non-loopback address without a strong `api_key`. Infrarust will refuse to start in that case rather than serve an unauthenticated admin API.
:::

### `[permissions]`

Maps players to the admin permission level and grants Player-level access to specific `/ir` subcommands. The model has two levels only, Player and Admin, with Player < Admin.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `admins` | array of strings | `[]` | Players granted the Admin level (by name or UUID) |
| `player_commands` | array of strings | `[]` | `/ir` subcommands made available to all players, not just admins |

```toml
[permissions]
admins = ["Notch", "00000000-0000-0000-0000-000000000000"]
player_commands = ["list", "find"]
```

### `[plugins.<id>]`

Per-plugin configuration, keyed by plugin ID. Plugins are WASM components; if `path` is omitted the plugin is discovered from `plugins_dir`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `path` | string | none | Explicit path to the plugin `.wasm` file. Omit to load it from `plugins_dir` |
| `permissions` | array of strings | `[]` | Permissions granted to this plugin |
| `enabled` | boolean | none | Whether the plugin is enabled. Treated as enabled when unset |

```toml
[plugins.auth]
path = "./plugins/auth.wasm"
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
| `addresses` | array | **required** | Backend addresses, each either `"host:port"` or `{ address = "host:port", weight = 3 }`. Port defaults to 25565 if omitted. Ordered by `balance` |
| `balance` | string | `"first_available"` | Address selection strategy: `"first_available"`, `"round_robin"`, or `"least_conn"` |
| `slow_start` | duration | none | Ramp a freshly healthy address up to full weight over this window, e.g. `"45s"` |
| `slow_start_aggression` | float | `1.0` | Slow start curve. `1.0` is linear, higher is gentler at the start |
| `active_health` | table | inherits global | Per-server override of the global `[active_health]` block |
| `proxy_mode` | string | `"passthrough"` | How the proxy handles traffic. See [Proxy modes](#proxy-modes) |
| `forwarding_mode` | string | none | Per-server override of the global `[forwarding]` mode: `"none"`, `"bungee_cord"`, `"bungee_guard"`, or `"velocity"`. Inherits the global setting when omitted |
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

::: tip
`passthrough`, `zero_copy`, and `server_only` forward raw bytes after the handshake. The proxy cannot inspect or modify packets in these modes.

`client_only` and `offline` parse packets, which enables server switching and limbo handlers.
:::

Each mode has its own page under [Proxy modes](../configuration/proxy-modes/) with the full behavior and trade-offs.

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

IP-based access control using CIDR notation. This table can also be set globally in `infrarust.toml`; a per-server filter narrows access further for one backend.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `whitelist` | array of strings | `[]` | CIDR ranges to allow. An empty whitelist allows everything not blacklisted |
| `blacklist` | array of strings | `[]` | CIDR ranges to reject |

An IP is allowed when it is not in the `blacklist` and the `whitelist` is empty or the IP is in the `whitelist`. The blacklist always applies, so a blacklisted address inside a whitelisted range is still rejected. The two lists can be used together.

```toml
[ip_filter]
whitelist = ["192.168.1.0/24", "10.0.0.0/8"]
```

```toml
[ip_filter]
blacklist = ["203.0.113.0/24"]
```

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
plugins_dir = "./plugins"
max_connections = 500
connect_timeout = "5s"
worker_threads = 0
receive_proxy_protocol = false
so_reuseport = false
unknown_domain_behavior = "default_motd"
announce_proxy_commands = true

[rate_limit]
enabled = true
max_connections = 3
window = "10s"
status_max = 300
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

[forwarding]
mode = "velocity"
secret_file = "forwarding.secret"

[web]
enable_api = true
bind = "127.0.0.1:8080"
api_key = "a-strong-api-key-at-least-16-chars"

[permissions]
admins = ["Notch"]
player_commands = ["list", "find"]

[plugins.auth]
path = "./plugins/auth.wasm"
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
