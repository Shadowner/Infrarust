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
| `provider` | string | `"builtin"` | `"builtin"`, `"none"` (no ban checks) or the id of the plugin that provides bans |
| `file` | string | `"bans.json"` | Path to the JSON file storing active bans (built-in provider) |
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

`mode` accepts two aliases for backward compatibility: `"legacy"` maps to `bungee_cord` and `"modern"` maps to `velocity`. See [Proxy forwarding](../configuration/proxy-forwarding) for the protocol details. The former `bungeecord_channel` key and `[forwarding.channel_permissions]` sub-table still load but are ignored, with a warning: the BungeeCord plugin messaging channel moved to [`[plugin_messaging]`](#plugin-messaging).

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

Chooses who answers permission questions and configures the built-in provider: admins hold every permission node, and `player_commands` opens chosen `/ir` subcommands to everyone. Read at startup; changes need a restart. See [Permissions](../configuration/security/permissions).

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `provider` | string | `"builtin"` | `"builtin"`, or the id of the plugin that provides permissions |
| `admins` | array of strings | `[]` | Players holding `infrarust.admin` and every other node (by name or UUID) |
| `player_commands` | array of strings | `[]` | `/ir` subcommands made available to all players, not just admins; `"*"` for every one that may be opened |
| `trust_offline_admins` | bool | `false` | Let players who did not authenticate with Mojang be admins |

```toml
[permissions]
admins = ["Notch", "00000000-0000-0000-0000-000000000000"]
player_commands = ["list", "find"]
```

### `[events]`

Limits on plugin event listeners. A listener that panics is skipped and the event continues with the next listener.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `handler_timeout` | duration | `"10s"` | Longest an async listener may run for a regular event before it is cancelled |
| `slow_handler_threshold` | duration | `"1s"` | Listeners running longer than this are logged as slow |
| `packet_handler_timeout` | duration | `"10s"` | Longest an async raw packet listener may run before it is cancelled |
| `disconnect_deadline` | duration | `"15s"` | Longest the `DisconnectEvent` dispatch for one player may take, all listeners included. The player is removed from the registry when it ends or at the deadline. Also bounds how long a duplicate login waits for the previous session to end |

All four must be greater than zero. Synchronous listeners can't be cancelled; one that overruns is reported as slow.

```toml
[events]
handler_timeout = "10s"
slow_handler_threshold = "1s"
packet_handler_timeout = "10s"
disconnect_deadline = "15s"
```

### `[plugin_messaging]`

The `BungeeCord` plugin messaging channel (`bungeecord:main` since 1.13) that backend plugins use to ask the proxy things. See [Plugin messaging](../plugins/dev/messaging#the-bungeecord-channel).

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `bungeecord` | boolean | `false` | Answer BungeeCord channel requests from the servers that set `bungeecord_channel = true` |
| `bungeecord_permissions` | table | see below | Which subchannels are answered |

`bungeecord_permissions` has one boolean per subchannel: `connect`, `connect_other`, `ip`, `ip_other`, `player_count`, `player_list`, `get_servers`, `get_server`, `get_player_server`, `forward`, `forward_to_player`, `uuid`, `uuid_other`, `server_ip`, `message`, `message_raw`, `kick_player`, `kick_player_raw` (the subchannel names such as `KickPlayer` are accepted too). All are `true` except `connect_other`, `message`, `message_raw`, `kick_player` and `kick_player_raw`.

```toml
[plugin_messaging]
bungeecord = true

[plugin_messaging.bungeecord_permissions]
connect_other = true
```

### `[auth]`

Player authentication and identity.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `session_url` | string | `"https://sessionserver.mojang.com/session/minecraft/hasJoined"` | Session server `client_only` mode verifies joining players against. Any Yggdrasil-compatible `hasJoined` endpoint works |
| `offline_uuid` | string | `"offline"` | UUID of players the session server does not verify (offline and passthrough servers, `ForceOffline`). `"offline"`: always the name-based offline UUID. `"client"`: the UUID from the client's login start packet when it sends one (1.19.1+), otherwise the name-based offline UUID. Never random |

```toml
[auth]
session_url = "https://sessionserver.mojang.com/session/minecraft/hasJoined"
offline_uuid = "offline"
```

### `[wasm]`

Sandbox limits for every WASM plugin. Each plugin handles one call at a time; its calls wait in a queue of `queue_capacity` entries.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `epoch_tick` | duration | `"50ms"` | How often the sandbox clock ticks. CPU budgets are counted in ticks, rounded up. Proxy-wide only |
| `memory_limit_mb` | integer | `64` | Linear-memory cap per plugin, in MiB. Growing past it traps the plugin |
| `cpu_budget` | duration | `"3s"` | CPU time one call into a plugin may use before it traps. Waiting on a host call does not count |
| `codec_cpu_budget` | duration | `"800ms"` | CPU time for one codec filter call (`create`, `filter`, connection hooks) before it traps |
| `host_call_timeout` | duration | `"30s"` | Longest a ban-service or server-manager call made by a plugin may take. On expiry the plugin receives a `service-error` |
| `max_call_duration` | duration | `"60s"` | Wall-clock limit on one call into a plugin, host calls included. Past it the call is abandoned and the plugin's instance is replaced by a fresh one |
| `queue_capacity` | integer | `1024` | Calls that may wait for a busy plugin. A call arriving at a full queue is refused immediately and logged as a rate-limited warning |

Validation: `epoch_tick` must be between `1ms` and `1s`; `memory_limit_mb` between 1 and 4096; `cpu_budget` and `codec_cpu_budget` at least one `epoch_tick` and at most `1h`; `host_call_timeout` and `max_call_duration` greater than zero and at most `1h`; `queue_capacity` between 1 and 1048576. A `host_call_timeout` or `cpu_budget` longer than `max_call_duration` is accepted with a warning.

A call that has started runs to the end even if its caller stops waiting, so an event listener cut off by `[events] handler_timeout` is not a fault. A call still queued when its caller gives up is skipped.

```toml
[wasm]
epoch_tick = "50ms"
memory_limit_mb = 64
cpu_budget = "3s"
codec_cpu_budget = "800ms"
host_call_timeout = "30s"
max_call_duration = "60s"
queue_capacity = 1024
```

#### `[wasm.recovery]`

How the proxy recovers a WASM plugin after a fault (a trap, a call cut off by `max_call_duration`, or a panic in a host function during a call). The faulty instance is discarded and a fresh one is created from the compiled component, which runs `on_enable` again. See [Fault model](../plugins/wasm/fault-model).

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `max_restarts` | integer | `5` | Fresh instances the proxy starts straight away within `window`. One more fault quarantines the plugin |
| `window` | duration | `"5m"` | Sliding window over which restarts are counted |
| `backoff_initial` | duration | `"1s"` | How long the first quarantine lasts. Each quarantine in a row doubles it |
| `backoff_max` | duration | `"5m"` | Longest a quarantine lasts |

While a plugin is quarantined every call to it is answered at once without running guest code: events keep their result, commands do nothing, limbo handlers deny the player. When the backoff has passed the proxy tries a fresh instance again.

Validation: `max_restarts` at most 1000 (0 quarantines on the first fault); `window`, `backoff_initial` and `backoff_max` greater than zero and at most `24h`; `backoff_initial` no longer than `backoff_max`.

```toml
[wasm.recovery]
max_restarts = 5
window = "5m"
backoff_initial = "1s"
backoff_max = "5m"
```

### `[plugins.<id>]`

Per-plugin configuration, keyed by plugin ID. WASM plugins are discovered from `plugins_dir`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `path` | string | none | Accepted for compatibility. WASM plugins are always loaded from `plugins_dir` |
| `permissions` | array of strings | `[]` | Capabilities granted on top of the baseline |
| `deny` | array of strings | `[]` | Capabilities removed after the baseline and `permissions` are applied. A capability in both lists is denied. Also applies to compiled-in plugins |
| `strict_capabilities` | boolean | `false` | WASM plugins: refuse to load the plugin when it imports a host function whose capability it lacks, instead of loading it with those calls refused |
| `enabled` | boolean | `true` | Set to `false` to skip the plugin |

Capability names are kebab-case (`player-write`, `codec-filter`, ...); unknown names in either list are ignored with a warning. See [Capabilities & Sandbox](../plugins/wasm/capabilities#capability-matrix).

```toml
[plugins.auth]
permissions = ["limbo", "command"]
deny = ["player-write"]
enabled = true
```

#### `[plugins.<id>.wasm]`

Overrides the `[wasm]` limits for one plugin. Accepts `memory_limit_mb`, `cpu_budget`, `codec_cpu_budget`, `host_call_timeout`, `max_call_duration` and `queue_capacity` (not `epoch_tick`, which is proxy-wide), and a `recovery` table with any of the `[wasm.recovery]` keys. A key left out keeps the `[wasm]` value. The same validation applies to the resulting limits.

```toml
[plugins.auth.wasm]
memory_limit_mb = 128
max_call_duration = "10s"

[plugins.auth.wasm.recovery]
max_restarts = 2
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
| `bungeecord_channel` | boolean | `false` | Answer this server's BungeeCord channel requests when `[plugin_messaging] bungeecord` is on. Only for `offline` and `client_only` |

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

Available states: `online`, `sleeping`, `starting`, `crashed`, `stopping`, `unreachable`.

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

[events]
handler_timeout = "10s"
slow_handler_threshold = "1s"
packet_handler_timeout = "10s"
disconnect_deadline = "15s"

[wasm]
memory_limit_mb = 64
cpu_budget = "3s"
max_call_duration = "60s"
queue_capacity = 1024

[wasm.recovery]
max_restarts = 5
window = "5m"
backoff_initial = "1s"
backoff_max = "5m"

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
permissions = ["limbo", "command"]
deny = ["player-write"]
enabled = true

[plugins.auth.wasm]
memory_limit_mb = 128
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
