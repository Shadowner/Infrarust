---
title: Global Settings
description: Reference for infrarust.toml. Bind address, workers, timeouts, rate limits, keepalive, bans, forwarding, ip_filter, web admin API, permissions, WASM plugin limits, and other proxy-wide settings.
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

Maximum simultaneous client connections. `0` means unlimited. When the limit is reached, the proxy stops accepting connections until existing ones close: new clients wait in the operating system's backlog instead of being refused.

## Timeouts

```toml
connect_timeout = "5s"
```

How long the proxy waits when opening a TCP connection to a backend server. If the backend doesn't respond within this window, the connection attempt fails and the player sees an error.

```toml
connect_max_attempts = 3
```

How many of a server's backend addresses the proxy tries before giving up, which bounds worst-case login latency to `connect_max_attempts × connect_timeout`. Set it to `0` to try every address. See [Load balancing](./load-balancing).

All duration fields accept human-readable strings: `"5s"`, `"30s"`, `"1m"`, `"2m30s"`.

## Backend health probing

```toml
[active_health]
enabled = true
kind = "tcp"
unhealthy_interval = "10s"
probe_healthy = false
interval = "30s"
timeout = "3s"
max_concurrent = 8
```

Background probing that brings ejected backend addresses back into rotation. Recovery probing is on by default and only touches addresses that are currently ejected, so it costs nothing while everything is healthy. Every key is documented in [Load balancing](./load-balancing), and any server can override the whole block.

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

When `true` (the default), the proxy adds its built-in `/ir` command tree and the plugin commands each player may run to the Minecraft command graph packet the backend sends, and replaces any backend command with the same name. When a plugin registers or removes a command, connected players get the updated tree right away. Set to `false` to hide the proxy command suggestions from the player's tab completion; the commands still run.

## Proxy protocol

```toml
receive_proxy_protocol = false
```

When `true`, the proxy expects incoming connections to start with a HAProxy PROXY protocol header (v1 or v2). Enable this if Infrarust sits behind a load balancer that sends proxy protocol, such as HAProxy or AWS NLB. The address from the header is the player's address everywhere: IP bans and filters, `PreLoginEvent.remote_addr` and `Player::remote_addr()`.

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
provider = "builtin"
file = "bans.json"
purge_interval = "300s"
enable_audit_log = true

[events]
handler_timeout = "10s"
slow_handler_threshold = "1s"
packet_handler_timeout = "10s"
disconnect_deadline = "15s"
```

`provider` picks who decides who is banned: `"builtin"` (the default) uses the ban file, `"none"` turns ban checks off, and a plugin id hands bans to that plugin. See [Bans](./security/bans#choosing-a-provider). `file` is the path to the JSON file where the built-in provider stores bans. `purge_interval` controls how often expired bans are removed from the file. When `enable_audit_log` is `true`, every ban and unban operation is logged.

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
| `bungee_cord` / `legacy` | BungeeCord-style legacy forwarding in the handshake |
| `bungee_guard` | BungeeCord forwarding with a shared HMAC token |
| `velocity` / `modern` | Velocity modern forwarding (recommended if your backends support it) |

`secret_file` is the path to the shared secret used by `bungee_guard` and `velocity`. The file is created automatically if it does not exist.

`bungeecord_channel` enables the `BungeeCord` plugin messaging channel. The `[forwarding.channel_permissions]` subtable controls which sub-channels are allowed; most are enabled by default, and `connect_other`, `message`, `message_raw`, `kick_player`, and `kick_player_raw` are disabled by default.

::: warning
BungeeCord legacy forwarding sends the real IP in plain text in the handshake. Anyone who can reach your backend port can spoof it. Use `bungee_guard` or `velocity` if you need IP forwarding and cannot fully firewall the backend.
:::

## Authentication

```toml
[auth]
session_url = "https://sessionserver.mojang.com/session/minecraft/hasJoined"
offline_uuid = "offline"
```

`session_url` is the endpoint `client_only` mode calls to verify a joining player. It defaults to Mojang's, so you only set it when your accounts live somewhere else.

Point it at an [authlib-injector](https://github.com/yushijinhun/authlib-injector) deployment or any other Yggdrasil-compatible server to authenticate against that instead. Give the complete endpoint URL rather than just the host, since implementations differ in how they prefix their routes — most authlib-injector servers expose it under `/authlib-injector/sessionserver/session/minecraft/hasJoined`.

The setting has no effect in any other proxy mode: `offline` never authenticates, and the forwarding modes leave authentication to the backend.

`offline_uuid` decides the UUID of a player who is not verified by the session server: every player on an `offline` or passthrough server, and a `client_only` player let in with `ForceOffline`.

| Value | UUID |
|-------|------|
| `"offline"` (default) | The name-based offline UUID, the one a vanilla server in offline mode computes: an MD5 UUID of `OfflinePlayer:<name>`. The same name always gets the same UUID |
| `"client"` | The UUID the client sends in its login start packet (1.19.1 and later). Clients that send none get the name-based offline UUID |

With `"offline"` a client cannot choose its own UUID. Use `"client"` only when the connection comes from something you trust to set it, such as another proxy in front of Infrarust. The UUID is never random: it is what plugins see in `PreLoginEvent`, `PostLoginEvent` and the player registry, what UUID bans match, and what forwarding sends to the backend.

::: warning
Every player who reaches a `client_only` server is verified against this URL. Pointing it at a server you do not control means letting that server decide who may join.
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

Enables the HTTP admin API (and optional web UI) used by management tools and the CLI. The section is optional; omit it entirely to keep the web interface off. `enable_webui` follows `enable_api` when you do not set it, so `enable_api = false` on its own turns both off. Setting `enable_webui = true` next to `enable_api = false` is rejected at startup, since the dashboard is served by the API's own HTTP server and calls it for every screen.

`bind` defaults to `127.0.0.1:8080`. If you bind to a non-loopback address, `api_key` is required and must be at least 16 characters. When bound to loopback without a key, Infrarust generates an ephemeral key and logs it at startup.

`cors_origins` accepts a list of allowed CORS origins (empty by default, meaning no cross-origin access).

The API can rewrite this file through `PUT /api/v1/config/proxy/raw`. Nothing is hot-applied: the proxy keeps running on the configuration it started with until you restart it. Reads replace `api_key` with `<redacted>` and a write puts the stored value back, so editing the config through the dashboard cannot destroy the key. A write is refused when it carries `<redacted>` and there is no stored key to restore, and the proxy refuses to start if `<redacted>` ever reaches the file itself. See [Admin API & Web UI](../plugins/builtin/admin-api).

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

## Plugin event handlers

```toml
[events]
handler_timeout = "10s"
slow_handler_threshold = "1s"
packet_handler_timeout = "10s"
disconnect_deadline = "15s"
```

Limits on the event listeners that plugins register. A listener that panics is skipped and the event moves on to the next listener, so a buggy plugin can't take down a player's connection or the proxy. Whatever the listener changed on the event before it panicked is kept.

`handler_timeout` caps how long one async listener may run for a regular event such as `PreLoginEvent` or `ChatMessageEvent`. When it runs out, the proxy cancels that listener and continues with the next one, so a stuck plugin delays a login by this much at most. `packet_handler_timeout` does the same for raw packet listeners.

`slow_handler_threshold` logs a warning for any listener that takes longer than this. Synchronous listeners can't be interrupted, so one that runs past `handler_timeout` finishes anyway and shows up as slow rather than timed out.

`disconnect_deadline` bounds the whole `DisconnectEvent` dispatch for one player, every listener included. When a player leaves, the proxy runs the `DisconnectEvent` listeners and removes the player from the registry once they are done, or once this deadline passes, whichever comes first. Listeners still running at the deadline are cancelled and a warning is logged. The same deadline bounds how long a second login with the same UUID waits for the first session to finish its `DisconnectEvent`. See [the player lifecycle](../plugins/dev/events#player-lifecycle).

Panics and timeouts are logged at error level, slow listeners at warn level, and each log line names the plugin and the event. All four values must be greater than zero.

## WASM plugin sandbox

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

Limits that apply to every WASM plugin. Each plugin runs in its own sandbox and handles one call at a time: its event listeners, commands, scheduled tasks and limbo callbacks wait in a queue and run in order.

| Key | What it limits |
|-----|----------------|
| `epoch_tick` | How often the sandbox clock ticks. CPU budgets are counted in ticks and rounded up to a whole number of them. Proxy-wide only. |
| `memory_limit_mb` | Linear memory of one plugin, in MiB. A plugin that grows past it traps. |
| `cpu_budget` | CPU time one call into a plugin may use before it traps. Time spent waiting on a host call (a ban lookup, a server start) does not count. |
| `codec_cpu_budget` | The same budget for each codec filter call (`create`, `filter` and the connection hooks). |
| `host_call_timeout` | How long one ban-service or server-manager call made by a plugin may take. When it runs out the plugin gets a `service-error` and carries on. A host call also ends early, with the same error, shortly before the deadline of the call it belongs to (`[events] handler_timeout` for an event, `max_call_duration` for a command, a scheduled task or a limbo callback), so the plugin always gets to decide. |
| `max_call_duration` | Wall-clock limit on one call into a plugin, host calls included. A call still running at this limit is abandoned and the plugin's instance is replaced by a fresh one. |
| `queue_capacity` | How many calls may wait for a busy plugin. When the queue is full a new call is refused on the spot: an event gets no answer from that plugin and a command does nothing. The refusal is logged as a warning, at most once every 5 seconds per plugin. |

A plugin call that has started always runs to the end, even when its caller stops waiting. If the event bus gives up on a WASM listener after `[events] handler_timeout`, the event moves on without that plugin's answer, the call finishes inside the plugin, and the plugin stays healthy. A call that is still queued when its caller gives up is dropped without running.

A trap (a panic in the plugin, a memory or CPU overrun) does not disable the plugin for good. The proxy throws the faulty instance away, starts a fresh one from the compiled plugin and runs its `on_enable` again. Anything the plugin kept only in memory is lost; files in its data directory are kept. A plugin that keeps failing is quarantined for a while, as set in `[wasm.recovery]` below. See [Fault model](../plugins/wasm/fault-model).

Startup fails when a value is out of range:

- `epoch_tick` must be between `1ms` and `1s`.
- `memory_limit_mb` must be between 1 and 4096.
- `cpu_budget` and `codec_cpu_budget` must be at least one `epoch_tick` and at most `1h`.
- `host_call_timeout` and `max_call_duration` must be greater than zero and at most `1h`.
- `queue_capacity` must be between 1 and 1048576.

The proxy logs a warning, without refusing to start, when `cpu_budget` is longer than `max_call_duration`: the wall-clock limit then stops a busy guest call first instead of the CPU budget trapping it.

### Recovery after a fault

```toml
[wasm.recovery]
max_restarts = 5
window = "5m"
backoff_initial = "1s"
backoff_max = "5m"
```

After a fault the proxy starts a fresh instance of the plugin straight away, up to `max_restarts` times within `window`. The next fault inside the window quarantines the plugin for `backoff_initial`, doubled for each quarantine in a row up to `backoff_max`. While a plugin is quarantined its events keep their result, its commands do nothing and its limbo handlers deny the player, all without waiting. Once the backoff has passed the proxy tries a fresh instance again.

| Key | What it controls |
|-----|------------------|
| `max_restarts` | Fresh instances started straight away within `window`. `0` quarantines the plugin on its first fault. |
| `window` | Sliding window over which restarts are counted. |
| `backoff_initial` | Length of the first quarantine. |
| `backoff_max` | Longest a quarantine lasts. |

Each fault is logged at error level with the plugin, the call and the cause. A successful recovery is logged at info level with the instance generation, and a quarantine at warn level with the time until the next attempt.

Startup fails when `max_restarts` is above 1000, when `window`, `backoff_initial` or `backoff_max` is zero or longer than `24h`, or when `backoff_initial` is longer than `backoff_max`.

## Plugins

```toml
[plugins.my_plugin]
permissions = ["ban", "limbo"]
deny = ["player-write"]
strict_capabilities = false
enabled = true

[plugins.my_plugin.wasm]
memory_limit_mb = 128
queue_capacity = 256
```

Plugin configurations are keyed by plugin ID.

- `permissions` grants capabilities on top of the baseline every WASM plugin receives.
- `deny` removes capabilities. It is applied after the baseline and the grants, so it can take away a baseline capability such as `player-write`, and a capability listed in both `permissions` and `deny` is denied. It also applies to compiled-in plugins.
- `strict_capabilities` (WASM plugins, defaults to `false`) refuses to load the plugin when it imports a host function whose capability it lacks. Without it such a plugin loads, a warning names each import that will be refused, and the calls are refused when made. See [What a missing capability does](../plugins/wasm/capabilities#what-a-missing-capability-does).
- `enabled` skips the plugin when set to `false` (defaults to `true` when omitted).
- `[plugins.<id>.wasm]` overrides the `[wasm]` limits for that plugin. It accepts every key of `[wasm]` except `epoch_tick`, and `[plugins.<id>.wasm.recovery]` overrides `[wasm.recovery]`; keys it leaves out keep the proxy-wide value.

Unknown capability names in `permissions` or `deny` are ignored with a warning. The capability strings are listed in [Capabilities & Sandbox](../plugins/wasm/capabilities#capability-matrix). `path` is accepted for compatibility; WASM plugins are always discovered in `plugins_dir`.

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

[wasm]
epoch_tick = "50ms"
memory_limit_mb = 64
cpu_budget = "3s"
codec_cpu_budget = "800ms"
host_call_timeout = "30s"
max_call_duration = "60s"
queue_capacity = 1024

[wasm.recovery]
max_restarts = 5
window = "5m"
backoff_initial = "1s"
backoff_max = "5m"

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
