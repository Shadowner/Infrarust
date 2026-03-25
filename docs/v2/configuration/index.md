---
title: Configuration Overview
description: How Infrarust configuration files are organized, their format, and how hot-reload works.
---

# Configuration Overview

Infrarust uses TOML files for all configuration. There are two levels: one global file for the proxy itself, and one file per backend server.

## File structure

```
.
├── infrarust.toml          # Global proxy settings
└── servers/                # One .toml file per backend server
    ├── survival.toml
    ├── creative.toml
    └── lobby.toml
```

`infrarust.toml` controls the proxy's listen address, rate limits, telemetry, bans, and other global behavior. Each file in the `servers/` directory defines a single backend server with its domains, address, proxy mode, and MOTD.

The `servers/` directory path is configurable via the `servers_dir` field in `infrarust.toml`. It defaults to `./servers`.

## Minimal setup

You need at least two files to get started: the global config and one server definition.

**infrarust.toml**

```toml
servers_dir = "./servers"
```

Every field in `infrarust.toml` has a default value, so an empty file (or even no file) works. The proxy listens on `0.0.0.0:25565` by default.

**servers/survival.toml**

```toml
domains = ["survival.example.com"]
addresses = ["127.0.0.1:25565"]
```

The only required field in a server file is `addresses`. Without `domains`, the server won't receive traffic from domain-based routing (but can still be reached via server switching).

## Global config: infrarust.toml

Here is a full example showing every section with its default values:

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

[default_motd.online]
text = "§cUnknown server"
version_name = "Infrarust"
max_players = 0

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
service_version = "2.0.0"
```

Durations use human-readable strings: `"5s"`, `"10m"`, `"1h"`. This applies to all timeout and interval fields.

For details on each section, see [Global Settings](./global.md).

## Server config files

Each `.toml` file in the servers directory defines one backend. The filename becomes the server's ID unless you set `id` or `name` explicitly.

```toml
domains = ["survival.mc.example.com", "*.survival.example.com"]
addresses = ["10.0.1.10:25565"]
proxy_mode = "passthrough"
send_proxy_protocol = false
domain_rewrite = "none"
max_players = 100

[timeouts]
connect = "3s"
read = "30s"
write = "30s"

[motd.online]
text = "§aSurvival §7— §fWelcome!"
favicon = "./icons/survival.png"

[motd.sleeping]
text = "§eSurvival §7— §fConnect to wake up!"
version_name = "Server Sleeping"
```

Domains support wildcard patterns. `*.mc.example.com` matches `survival.mc.example.com`, `creative.mc.example.com`, and so on. Exact domain matches take priority over wildcards.

Available proxy modes: `passthrough` (default), `zerocopy`, `client_only`, `offline`, `server_only`. See [Proxy Modes](./proxy-modes/) for what each one does.

For the full list of server config fields, see [Server Definitions](./servers.md).

## Hot-reload

Infrarust watches the `servers_dir` directory for changes. When you add, edit, or remove a `.toml` file, the proxy picks up the change automatically. No restart required.

The file watcher uses 200ms debouncing to avoid reacting to partial writes. After the debounce window, it computes a diff against the known configuration and applies only what changed:

- New file added: server becomes routable immediately
- File modified: routing and settings update in place
- File deleted: server is removed from the router

::: tip
Only server config files (in `servers_dir`) are hot-reloaded. Changes to `infrarust.toml` require a proxy restart.
:::

The status cache is automatically invalidated when a server's configuration changes, so players see updated MOTDs right away.

## Strict parsing

Both `infrarust.toml` and server files use strict parsing (`deny_unknown_fields`). If you misspell a key or add a field that doesn't exist, the proxy will reject the file with a clear error message instead of silently ignoring it.

## Configuration providers

The file provider described above is always active, but Infrarust also supports other configuration sources:

- **Docker provider** discovers servers from running containers using labels. See [Docker Discovery](./providers/docker.md).
- **Plugins** can register as configuration providers to supply server definitions from external sources.

All providers feed into the same routing system. A server defined via Docker labels works the same way as one defined in a `.toml` file.
