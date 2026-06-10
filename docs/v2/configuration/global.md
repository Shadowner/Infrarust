---
title: Global Settings
description: Reference for infrarust.toml. Bind address, workers, timeouts, rate limits, keepalive, bans, forwarding, ip_filter, web admin API, permissions, and other proxy-wide settings.
outline: [2, 3]
---

# Global Settings

The `infrarust.toml` file controls the proxy process itself: what address it listens on, how many threads it uses, and how it handles connections before they reach any backend server.

Every field has a default value. An empty file (or no file at all) starts the proxy on `0.0.0.0:25565` with sane defaults.

The config struct uses `serde(deny_unknown_fields)`, so unrecognized keys are a hard parse error rather than a silent no-op.

## Bind address and port

```toml
bind = "0.0.0.0:25565"
```

The socket address the proxy listens on. The format is `ip:port`. Set the IP to `127.0.0.1` to accept connections only from localhost, or `0.0.0.0` to accept from any interface.

To run on a non-standard port:

```toml
bind = "0.0.0.0:25577"
```

## Worker threads

```toml
worker_threads = 0
```

Number of Tokio async runtime threads. `0` (the default) lets the runtime pick a count based on available CPU cores. Set this explicitly if you want to cap CPU usage on a shared host.

## Connection limits

```toml
max_connections = 0
```

Maximum simultaneous client connections. `0` means unlimited. When the limit is reached, new connections are rejected until existing ones close.

## Timeouts

```toml
connect_timeout = "5s"
```

How long the proxy waits when opening a TCP connection to a backend server. If the backend doesn't respond within this window, the connection attempt fails and the player sees an error.

All duration fields accept human-readable strings: `"5s"`, `"30s"`, `"1m"`, `"2m30s"`.

## Server and plugin directories

```toml
servers_dir = "./servers"
plugins_dir = "./plugins"
```

`servers_dir` is the path to the directory containing per-server `.toml` files. `plugins_dir` is where Infrarust looks for WASM plugin files. Both are resolved from the working directory where Infrarust starts. See the [Configuration Overview](./) for the per-server config format.

## Announce proxy commands

```toml
announce_proxy_commands = true
```

When `true` (the default), the proxy announces its built-in `/ir` command tree to clients via the Minecraft command graph packet. Set to `false` to hide the proxy command suggestions from the player's tab completion.

## Proxy protocol

```toml
receive_proxy_protocol = false
```

When `true`, the proxy expects incoming connections to start with a HAProxy PROXY protocol header (v1 or v2). Enable this if Infrarust sits behind a load balancer that sends proxy protocol, such as HAProxy or AWS NLB.

::: warning
Only enable this if your upstream actually sends proxy protocol headers. Regular Minecraft clients do not, and connections will fail if this is on without a proxy protocol source.
:::

## SO_REUSEPORT

```toml
so_reuseport = false
```

Enables the `SO_REUSEPORT` socket option, which allows multiple processes to bind to the same port. This is a Linux-only option and has no effect on other platforms. Useful when running multiple Infrarust instances behind a kernel-level load balancer.

## Unknown domain behavior

```toml
unknown_domain_behavior = "default_motd"
```

What happens when a player connects with a domain that doesn't match any server definition.

| Value | Behavior |
|-------|----------|
| `default_motd` | Respond with the MOTD defined in `[default_motd]` (default) |
| `drop` | Close the connection silently |

## Rate limiting

```toml
[rate_limit]
enabled = false
max_connections = 3
window = "10s"
status_max = 300
status_window = "10s"
```

Controls how many connections a single IP can make within a sliding time window. Rate limiting is disabled by default; set `enabled = true` to activate it. Login attempts and status pings have separate limits.

`max_connections` is the number of login attempts allowed per IP within `window`. `status_max` and `status_window` do the same for server-list ping requests. The defaults allow 3 login attempts and 300 status pings per 10-second window per IP.

## Status cache

```toml
[status_cache]
ttl = "5s"
max_entries = 1000
```

The proxy caches server-list ping responses to avoid hammering backend servers. `ttl` is how long a cached response stays valid. `max_entries` caps the cache size.

If you run many backend servers and see stale ping data, lower the `ttl`. If memory is a concern, lower `max_entries`.

## TCP keepalive

```toml
[keepalive]
time = "30s"
interval = "10s"
retries = 3
```

TCP keepalive probes detect dead connections at the OS level. After a connection sits idle for `time`, the OS sends a probe every `interval`. After `retries` failed probes, the connection is closed.

These values apply to both player-to-proxy and proxy-to-backend connections.

## Ban system

```toml
[ban]
file = "bans.json"
purge_interval = "300s"
enable_audit_log = true
```

`file` is the path to the JSON file where bans are stored. `purge_interval` controls how often expired bans are removed from the file. When `enable_audit_log` is `true`, every ban and unban operation is logged.

## Default MOTD

```toml
[default_motd.online]
text = "§cUnknown server"
version_name = "Infrarust"
max_players = 0
```

The MOTD shown when a player pings a domain that doesn't match any server. You can set different MOTDs for different states: `online`, `offline`, `sleeping`, `starting`, `crashed`, `stopping`, `unreachable`.

Each MOTD entry supports these fields:

| Field | Type | Description |
|-------|------|-------------|
| `text` | string | MOTD text, supports Minecraft `§` formatting codes |
| `favicon` | string | Path to a 64x64 PNG, a base64 string, or a URL |
| `version_name` | string | Version text shown in the client |
| `max_players` | integer | Max player count displayed in the server list |

## Telemetry

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

Infrarust can export metrics and traces via OpenTelemetry. The `[telemetry]` section is absent by default (no telemetry); add it and set `enabled = true` to activate export. Point `endpoint` at your OTLP collector; when omitted, the OpenTelemetry SDK default is used.

`protocol` is either `"grpc"` or `"http"`, matching the OTLP export protocol your collector expects.

`sampling_ratio` controls what fraction of status ping traces are sampled (0.0 to 1.0). Login traces are always sampled at 100% regardless of this value.

`service_name` is set as an OTEL resource attribute. `service_version` defaults to the Infrarust binary version and is usually left unset.

::: tip
Omitting the `[telemetry]` section entirely disables telemetry. No collector connection is attempted.
:::

## Docker provider

```toml
[docker]
endpoint = "unix:///var/run/docker.sock"
poll_interval = "30s"
reconnect_delay = "5s"
```

Enables automatic server discovery from Docker container labels. `endpoint` is the Docker daemon socket or HTTP API URL. `network` (optional) specifies which Docker network to use when resolving container addresses.

The provider uses Docker events for real-time updates and falls back to polling every `poll_interval` if the event stream disconnects. After a disconnect, it waits `reconnect_delay` before reconnecting.

<!-- Link to Docker discovery docs when available -->

::: info
The `[docker]` section is optional. Omit it entirely to disable Docker discovery.
:::

## IP filter

```toml
[ip_filter]
whitelist = ["10.0.0.0/8", "192.168.0.0/16"]
blacklist = ["10.6.6.0/24"]
```

Global IP filtering using CIDR ranges. An IP is allowed when it is not in the `blacklist` and (the `whitelist` is empty or the IP is in the `whitelist`). The blacklist always wins: a blacklisted address inside a whitelisted range is still rejected.

If `whitelist` is empty, all IPs are allowed except those in `blacklist`. Both lists can be used together.

Individual servers can define their own `[ip_filter]` in addition to, or instead of, the global one.

## Forwarding

```toml
[forwarding]
mode = "none"
secret_file = "forwarding.secret"
bungeecord_channel = true
```

Player IP forwarding passes the real client IP and UUID to backend servers. The `mode` values are:

| Mode | Description |
|------|-------------|
| `none` | No forwarding (default) |
| `bungeecord` / `legacy` | BungeeCord-style legacy forwarding in the handshake |
| `bungeeguard` | BungeeCord forwarding with a shared HMAC token |
| `velocity` / `modern` | Velocity modern forwarding (recommended if your backends support it) |

`secret_file` is the path to the shared secret used by `bungeeguard` and `velocity`. The file is created automatically if it does not exist.

`bungeecord_channel` enables the `BungeeCord` plugin messaging channel. The `[forwarding.channel_permissions]` subtable controls which sub-channels are allowed; most are enabled by default, and `connect_other`, `message`, `message_raw`, `kick_player`, and `kick_player_raw` are disabled by default.

::: warning
BungeeCord legacy forwarding sends the real IP in plain text in the handshake. Anyone who can reach your backend port can spoof it. Use `bungeeguard` or `velocity` if you need IP forwarding and cannot fully firewall the backend.
:::

## Web admin API

```toml
[web]
enable_api = true
enable_webui = true
bind = "127.0.0.1:8080"
api_key = "your-api-key-here"

[web.rate_limit]
requests_per_minute = 60
```

Enables the HTTP admin API (and optional web UI) used by management tools and the CLI. The section is optional; omit it entirely to keep the web interface off.

`bind` defaults to `127.0.0.1:8080`. If you bind to a non-loopback address, `api_key` is required and must be at least 16 characters. When bound to loopback without a key, Infrarust generates an ephemeral key and logs it at startup.

`cors_origins` accepts a list of allowed CORS origins (empty by default, meaning no cross-origin access).

::: danger
Never expose the admin API on a public interface without a strong `api_key`. There is no second authentication layer.
:::

## Permissions

```toml
[permissions]
admins = ["PlayerName", "AnotherPlayer"]
player_commands = []
```

`admins` is a list of player names (or UUIDs) granted the Admin permission level. Admin players can run all `/ir` subcommands including `broadcast`, `kick`, `reload`, `send`, `plugin`, and `plugins`.

`player_commands` overrides which `/ir` subcommands non-admin players can run. By default, players can use `help`, `version`, `list`, `find`, and `server`.

Plugins can register custom permission checkers that extend or replace this list.

## Plugins

```toml
[plugins.my_plugin]
path = "./plugins/my_plugin.wasm"
permissions = ["event_handler"]
enabled = true
```

Plugin configurations are keyed by plugin ID. Each entry can specify a `path` to the plugin binary, a list of `permissions`, and whether the plugin is `enabled` (defaults to `true` when omitted). WASM plugins are loaded from `plugins_dir` by default; `path` overrides the location for that specific plugin.

## Full example

A complete `infrarust.toml` showing all sections and their defaults:

```toml
bind = "0.0.0.0:25565"
max_connections = 0
connect_timeout = "5s"
receive_proxy_protocol = false
servers_dir = "./servers"
plugins_dir = "./plugins"
worker_threads = 0
unknown_domain_behavior = "default_motd"
so_reuseport = false
announce_proxy_commands = true

[rate_limit]
enabled = false
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

# [telemetry]
# enabled = true
# endpoint = "http://localhost:4317"
# protocol = "grpc"
#
# [telemetry.metrics]
# enabled = true
# export_interval = "15s"
#
# [telemetry.traces]
# enabled = true
# sampling_ratio = 0.1
#
# [telemetry.resource]
# service_name = "infrarust"

# [ip_filter]
# whitelist = []
# blacklist = []

# [forwarding]
# mode = "none"
# secret_file = "forwarding.secret"

# [web]
# bind = "127.0.0.1:8080"
# api_key = "change-me-min-16-chars"

# [permissions]
# admins = []
```
