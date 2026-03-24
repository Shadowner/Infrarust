---
title: Global Settings
description: Reference for infrarust.toml — bind address, workers, timeouts, rate limits, keepalive, bans, and other proxy-wide settings.
outline: [2, 3]
---

# Global Settings

The `infrarust.toml` file controls the proxy process itself: what address it listens on, how many threads it uses, and how it handles connections before they reach any backend server.

Every field has a default value. An empty file (or no file at all) starts the proxy on `0.0.0.0:25565` with sane defaults.

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

## Server directory

```toml
servers_dir = "./servers"
```

Path to the directory containing per-server `.toml` files. Relative paths are resolved from the working directory where Infrarust starts. See the [Configuration Overview](./) for the per-server config format.

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
max_connections = 3
window = "10s"
status_max = 30
status_window = "10s"
```

Controls how many connections a single IP can make within a sliding time window. Login attempts and status pings have separate limits.

`max_connections` is the number of login attempts allowed per IP within `window`. `status_max` and `status_window` do the same for server-list ping requests.

The defaults allow 3 login attempts and 30 status pings per 10-second window per IP. Set `max_connections = 0` to disable login rate limiting.

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
enabled = false
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

Infrarust can export metrics and traces via OpenTelemetry. Set `enabled = true` and point `endpoint` at your OTLP collector. When `endpoint` is omitted, the OpenTelemetry SDK default is used.

`protocol` is either `"grpc"` or `"http"`, matching the OTLP export protocol your collector expects.

`sampling_ratio` controls what fraction of status ping traces are sampled (0.0 to 1.0). Login traces are always sampled at 100% regardless of this value.

`service_name` is set as an OTEL resource attribute. `service_version` defaults to the Infrarust binary version and is usually left unset.

::: tip
Telemetry is fully disabled by default. Setting `enabled = false` (or omitting the section entirely) means no collector connection is attempted.
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

## Plugins

```toml
[plugins.my_plugin]
path = "/opt/infrarust/plugins/my_plugin.so"
permissions = ["config_provider", "event_handler"]
enabled = true
```

Plugin configurations are keyed by plugin ID. Each entry can specify a `path` to the plugin binary, a list of `permissions`, and whether the plugin is `enabled` (defaults to `true` when omitted).

## Full example

A complete `infrarust.toml` with every section and its defaults:

```toml
bind = "0.0.0.0:25565"
max_connections = 0
connect_timeout = "5s"
receive_proxy_protocol = false
servers_dir = "./servers"
worker_threads = 0
unknown_domain_behavior = "default_motd"
so_reuseport = false

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

[telemetry]
enabled = false
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
