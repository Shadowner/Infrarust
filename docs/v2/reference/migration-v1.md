---
title: Migration from V1
description: How to upgrade from Infrarust V1 (YAML) to V2 (TOML), using the built-in migrate command and a field-by-field mapping of changed options.
---

# Migration from V1

V2 replaces the YAML configuration format with TOML and reorganizes several options. The `infrarust migrate` command converts V1 files automatically, but some settings require manual attention after the conversion.

## Running the migration

The `migrate` subcommand reads your V1 server YAML files and writes V2 TOML files to an output directory. It can also convert your global `config.yaml` at the same time.

```bash
infrarust migrate <INPUT_DIR> [OPTIONS]
```

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--output <DIR>` | `-o` | `./servers` | Directory to write converted TOML files |
| `--config <PATH>` | | | Path to a V1 `config.yaml` to convert alongside server configs |

Migrate server configs only:

```bash
infrarust migrate ./proxies
```

Migrate server configs and the global config together:

```bash
infrarust migrate ./proxies --output ./servers --config ./config.yaml
```

If you pass `--config`, the tool writes the converted global config to `<OUTPUT>/infrarust.toml`. Each server YAML file becomes a TOML file with the same base name in the output directory.

The tool prints each warning or error it encounters, labelled `INFO`, `WARN`, or `ERROR`. Errors mean the converted file is incomplete and needs manual fixes before the proxy will accept it. The command exits with a non-zero code if any error occurred.

::: tip
Review every `WARN` and `ERROR` line before starting V2 for the first time. The converted files are a starting point, not a guaranteed drop-in replacement.
:::

## File layout change

V1 used a flat layout where `config.yaml` pointed to server files via `file_provider.proxies_path`:

```
infrarust/
├── config.yaml
└── proxies/
    ├── hub.yml
    └── survival.yml
```

V2 drops the `file_provider` wrapper. The global config is `infrarust.toml`, and `servers_dir` points directly to the folder containing server TOML files:

```
infrarust/
├── infrarust.toml
└── servers/
    ├── hub.toml
    └── survival.toml
```

If `proxies_path` listed more than one directory in V1, the migrator uses the first path and warns that the others are ignored. V2 supports only one `servers_dir`.

## Global config (config.yaml → infrarust.toml)

The table below shows which V1 top-level keys map to V2. Items marked **dropped** have no equivalent; items marked **moved** appear under a different key or table.

| V1 key | V2 equivalent | Notes |
|--------|---------------|-------|
| `bind` | `bind` | Same syntax |
| `keepAliveTimeout` | `[keepalive].time` | Only `time` is migrated; `interval` and `retries` take defaults |
| `file_provider.proxies_path[0]` | `servers_dir` | Only the first path is used |
| `file_provider.watch` | N/A | Hot-reload always works in V2 |
| `proxy_protocol.receive_enabled` | `receive_proxy_protocol` | Boolean, top-level |
| `proxy_protocol.receive_timeout_secs` | dropped | Not in V2 |
| `proxy_protocol.receive_allowed_versions` | dropped | V1 and V2 are both accepted automatically |
| `cache.status_ttl_seconds` | `[status_cache].ttl` | In seconds, same meaning |
| `cache.max_status_entries` | `[status_cache].max_entries` | Same meaning |
| `filters.rate_limiter.request_limit` | `[rate_limit].max_connections` | |
| `filters.rate_limiter.window_length` | `[rate_limit].window` | humantime string |
| `filters.ip_filter` | per-server `[ip_filter]` | Global IP filter moved to per-server config |
| `filters.id_filter` | dropped | Not available in V2 |
| `filters.name_filter` | dropped | Not available in V2 |
| `filters.ban` | `[ban]` | Migrated field-by-field |
| `telemetry.enabled` | `[telemetry].enabled` | |
| `telemetry.export_url` | `[telemetry].endpoint` | |
| `telemetry.export_interval_seconds` | `[telemetry].metrics.export_interval` | |
| `telemetry.enable_metrics` | `[telemetry].metrics.enabled` | |
| `telemetry.enable_tracing` | `[telemetry].traces.enabled` | |
| `logging` | dropped | Use `RUST_LOG` env var or `--log-level` CLI flag |
| `managers_config` | per-server `[server_manager]` | Credentials moved to each server file |
| `docker_provider.docker_host` | `[docker].endpoint` | |
| `docker_provider.polling_interval` | `[docker].poll_interval` | In seconds |
| `motds.unreachable` | `[default_motd]` | Used as the fallback MOTD for unknown domains |

### Example

V1 `config.yaml`:

```yaml
bind: "0.0.0.0:25565"
keepAliveTimeout: 30s

file_provider:
  proxies_path: ["./proxies"]
  watch: true

cache:
  status_ttl_seconds: 5
  max_status_entries: 1000

filters:
  rate_limiter:
    enabled: true
    request_limit: 3
    window_length: 10s

telemetry:
  enabled: false
  export_url: "http://localhost:4317"
  export_interval_seconds: 15
  enable_metrics: true
  enable_tracing: true
```

Equivalent V2 `infrarust.toml`:

```toml
bind = "0.0.0.0:25565"
servers_dir = "./servers"

[keepalive]
time = "30s"

[status_cache]
ttl = "5s"
max_entries = 1000

[rate_limit]
max_connections = 3
window = "10s"

[telemetry]
enabled = false
endpoint = "http://localhost:4317"

[telemetry.metrics]
enabled = true
export_interval = "15s"

[telemetry.traces]
enabled = true
```

## Server configs (proxies/*.yml → servers/*.toml)

### Field mapping

| V1 key | V2 key | Notes |
|--------|--------|-------|
| `domains` | `domains` | Same format, supports wildcards |
| `addresses` | `addresses` | Same `host:port` format |
| `proxyMode` | `proxy_mode` | Values renamed (see below) |
| `sendProxyProtocol` | `send_proxy_protocol` | |
| `proxy_protocol_version` | dropped | V2 auto-detects the version |
| `config_id` | `name` | Sanitized to `[a-z0-9_-]+` |
| `backend_domain` | `domain_rewrite` | See rewrite section below |
| `rewrite_domain: true` | `domain_rewrite = "from_backend"` | |
| `motds` | `[motd]` | State names partly changed (see below) |
| `filters.ip_filter` | `[ip_filter]` | Migrated, whitelist/blacklist preserved |
| `filters.rate_limiter` | now global | INFO: move to `infrarust.toml [rate_limit]` |
| `filters.id_filter` | dropped | WARN: not available in V2 |
| `filters.name_filter` | dropped | WARN: not available in V2 |
| `filters.ban` | now global | INFO: move to `infrarust.toml [ban]` |
| `caches` | now global | INFO: move to `infrarust.toml [status_cache]` |
| `server_manager` | `[server_manager]` | Credentials moved inline (see below) |

### Proxy mode names

V1 used camelCase values for `proxyMode`. V2 uses snake_case:

| V1 `proxyMode` | V2 `proxy_mode` |
|----------------|-----------------|
| `passthrough` | `passthrough` |
| `zerocopy_passthrough` | `zero_copy` |
| `client_only` | `client_only` |
| `server_only` | `server_only` |
| `offline` | `offline` |

### MOTD state names

V1 had several overlapping MOTD states. V2 consolidates them:

| V1 MOTD state | V2 MOTD state | Notes |
|---------------|---------------|-------|
| `online` | `online` | |
| `offline` | `sleeping` | Renamed to reflect server-sleep semantics |
| `starting` | `starting` | |
| `stopping` | `stopping` | |
| `shutting_down` | `stopping` | Merged; `shutting_down` takes priority if both exist |
| `crashed` | `crashed` | |
| `unreachable` | `unreachable` | |
| `unknown` | `unreachable` | Merged into `unreachable` |
| `unable_status` | `unreachable` | Merged into `unreachable` |

MOTD fields `protocol_version`, `online_players`, and `samples` are not in V2 and are dropped without error.

`max_players` from the `online` MOTD state is migrated to the server-level `max_players` field.

### Domain rewrite

V1 had two separate keys:

```yaml
rewrite_domain: true         # use the first backend address as the domain
backend_domain: "mc.internal.example.com"  # use this explicit domain
```

V2 uses a single `domain_rewrite` field with three possible values:

```toml
domain_rewrite = "none"                          # default
domain_rewrite = "from_backend"                  # was rewrite_domain = true
domain_rewrite = { explicit = "mc.internal.example.com" }  # was backend_domain
```

The migrator converts both forms automatically.

### Server manager

In V1, Pterodactyl and Crafty credentials lived in the global `managers_config` block. In V2, each server file contains its own credentials inside `[server_manager]`.

The migrator writes placeholder strings for `api_url` and `api_key` when it encounters a Pterodactyl or Crafty server manager, because those values are not present in the per-server V1 file:

```toml
[server_manager]
type = "pterodactyl"
api_url = "TODO: fill in your Pterodactyl panel URL"
api_key = "TODO: fill in your API key"
server_id = "your-server-uuid"
```

You must replace the `TODO` placeholders before the proxy will connect to the panel. The migrator emits a `WARN` line for each file affected.

For local servers, the migration is fully automatic:

```toml
[server_manager]
type = "local"
command = "java"
working_dir = "/path/to/server"
args = ["-jar", "server.jar"]
ready_pattern = 'For help, type "help"'
```

### Example

V1 `proxies/survival.yml`:

```yaml
domains:
  - "survival.example.com"
addresses:
  - "127.0.0.1:25565"
proxyMode: passthrough
sendProxyProtocol: false

motds:
  online:
    enabled: true
    text: "§aWelcome!"
    version_name: "Paper 1.21"
    max_players: 100
  offline:
    enabled: true
    text: "§eSleeping, connect to wake"
  unreachable:
    enabled: true
    text: "§cServer unreachable"

filters:
  ip_filter:
    enabled: true
    whitelist: []
    blacklist: ["10.0.0.5"]
```

V2 `servers/survival.toml` (after migration):

```toml
name = "survival"
domains = ["survival.example.com"]
addresses = ["127.0.0.1:25565"]
proxy_mode = "passthrough"
send_proxy_protocol = false
max_players = 100

[motd.online]
text = "§aWelcome!"
version_name = "Paper 1.21"

[motd.sleeping]
text = "§eSleeping, connect to wake"

[motd.unreachable]
text = "§cServer unreachable"

[ip_filter]
whitelist = []
blacklist = ["10.0.0.5/32"]
```

## What requires manual action

Some changes cannot be automated:

- Pterodactyl and Crafty credentials must be filled into each server file. Look for lines containing `TODO:` in the converted files.
- `id_filter` and `name_filter` have no V2 equivalent. If you relied on UUID or name filtering, you need to handle access control through a plugin or at the network level.
- Per-server rate limiting and cache settings are global in V2. Consolidate them into `infrarust.toml` under `[rate_limit]` and `[status_cache]`.
- Multiple `proxies_path` directories collapse to one `servers_dir`. If you split configs across directories in V1, merge them or restructure before migrating.
- The `logging` block has no V2 equivalent. Control verbosity with the `RUST_LOG` environment variable or the `--log-level` flag.

## After migration

1. Replace any `TODO:` placeholder values in the converted files.
2. Move per-server `rate_limiter` and `cache` config into `infrarust.toml`.
3. Start the proxy with `--log-level debug` on the first run to catch any remaining config problems early.
4. Check the [config schema](./config-schema) if a field name or valid value is unclear.
