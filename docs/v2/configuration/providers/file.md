---
title: File Provider
description: Configure Infrarust servers using TOML files in a directory, with automatic hot-reload on changes.
---

# File Provider

The file provider loads server configurations from `.toml` files in a directory. Each file defines one server. When you add, edit, or remove a file, Infrarust picks up the change automatically.

## Directory structure

By default, Infrarust looks for server configs in `./servers/` relative to where you run the binary. You can change this in `infrarust.toml`:

```toml
servers_dir = "./servers"
```

Place one `.toml` file per server inside that directory:

```
infrarust.toml
servers/
├── survival.toml
├── creative.toml
└── lobby.toml
```

Files without a `.toml` extension are ignored. Subdirectories are not scanned.

## Server config format

Each file deserializes into a `ServerConfig`. Here is a minimal example:

```toml
domains = ["survival.mc.example.com"]
addresses = ["10.0.1.10:25565"]
```

`domains` and `addresses` are the only fields you always need. Everything else has a default.

### Full example

```toml
# Domains that route to this server (supports wildcards)
domains = ["survival.mc.example.com", "*.survival.example.com"]

# Backend addresses (host:port, port defaults to 25565)
addresses = ["10.0.1.10:25565", "10.0.1.11:25565"]

# Proxy mode: passthrough, zero_copy, client_only, offline, server_only, full
proxy_mode = "passthrough"

# Forward PROXY protocol headers to the backend
send_proxy_protocol = false

# Rewrite the domain in the handshake: "none", "from_backend",
# or { explicit = "backend.local" } for a custom domain
domain_rewrite = "none"

# Max players shown in the server list (0 = unlimited)
max_players = 100

# Disconnect message when the backend is unreachable
disconnect_message = "Server is down. Try again later."

# Limbo handler chain (plugin IDs, executed in order)
limbo_handlers = ["queue", "auth"]

# Server-specific timeouts (override global values)
[timeouts]
connect = "3s"
read = "30s"
write = "30s"

# MOTD per server state
[motd.online]
text = "§aSurvival §7— §fWelcome!"
favicon = "./icons/survival.png"

[motd.sleeping]
text = "§eSurvival §7— §fConnect to wake up!"
version_name = "Server Sleeping"

[motd.starting]
text = "§eSurvival §7— §fStarting..."
```

### All fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `id` | string | filename stem | Unique identifier. Derived from the filename if absent. |
| `name` | string | none | Human-readable name. Takes priority over `id` for identification. Must match `[a-z0-9_-]+`. |
| `network` | string | none | Network group for server switching. Only servers in the same network can switch between each other. |
| `domains` | list of strings | `[]` | Domains that route to this server. Supports wildcards (`*.example.com`). |
| `addresses` | list of strings | required | Backend addresses in `host:port` format. Port defaults to `25565` if omitted. |
| `proxy_mode` | string | `"passthrough"` | One of: `passthrough`, `zero_copy`, `client_only`, `offline`, `server_only`, `full`. |
| `send_proxy_protocol` | bool | `false` | Send PROXY protocol to the backend. |
| `domain_rewrite` | string or table | `"none"` | Rewrite domain in handshake: `"none"`, `"from_backend"`, or `{ explicit = "domain" }`. |
| `max_players` | integer | `0` | Max players for status response. 0 means unlimited. |
| `disconnect_message` | string | `"Server is currently unreachable..."` | Message sent when backend is unreachable. |
| `limbo_handlers` | list of strings | `[]` | Plugin IDs for the limbo handler chain. |

#### Nested sections

`[timeouts]` overrides global timeout values for this server:

| Field | Type | Default |
|-------|------|---------|
| `connect` | duration | `"5s"` |
| `read` | duration | `"30s"` |
| `write` | duration | `"30s"` |

`[motd.<state>]` configures the MOTD for each server state (`online`, `offline`, `sleeping`, `starting`, `crashed`, `stopping`, `unreachable`):

| Field | Type | Default |
|-------|------|---------|
| `text` | string | required |
| `favicon` | string | none |
| `version_name` | string | none |
| `max_players` | integer | none |

`[server_manager]` configures automatic server start/stop. See [Server Manager](../servers.md) for details.

`[ip_filter]` restricts access by IP:

| Field | Type | Default |
|-------|------|---------|
| `whitelist` | list of CIDRs | `[]` |
| `blacklist` | list of CIDRs | `[]` |

## Server identification

Each server gets an ID from the file provider in the format `file@<filename>` (e.g., `file@survival.toml`). This is used internally by the provider registry.

The server's effective identity (used for display and server switching) follows this priority:

1. `name` field, if set
2. `id` field, if set
3. The filename without `.toml` extension (e.g., `survival` from `survival.toml`)

## Hot reload

The file provider watches the `servers_dir` directory using OS-level file notifications. When a file changes, Infrarust waits 200ms to batch rapid edits, then computes a diff against its in-memory state.

Three types of changes are detected:

- **New file** — a `.toml` file appears in the directory. The server is added to the router.
- **Modified file** — an existing file's content changes. The server config is updated.
- **Removed file** — a `.toml` file is deleted. The server is removed from the router.

::: warning
If you save a file with invalid TOML or a config that fails validation, the previous version stays active. Infrarust logs a warning but does not remove the server.
:::

You do not need to restart Infrarust after editing server configs. Add a new `.toml` file and players can connect to it within a second.

## Validation

Each file is validated on load. The following rules apply:

- Forwarding modes (`passthrough`, `zero_copy`, `server_only`) must have at least one domain.
- Forwarding modes cannot belong to a `network` (they don't support server switching).
- At least one address must be defined.
- Domain strings must not be empty.
- `name` and `network` must match `[a-z0-9_-]+` and be at most 64 characters.
- No two servers can share the same effective ID.

::: danger
The config uses `deny_unknown_fields`. A typo in a field name (e.g., `adresses` instead of `addresses`) will cause the file to be rejected entirely.
:::

## Example: multi-server setup

```
infrarust.toml
servers/
├── lobby.toml
├── survival.toml
└── creative.toml
```

::: code-group

```toml [lobby.toml]
domains = ["mc.example.com"]
addresses = ["127.0.0.1:25565"]
proxy_mode = "client_only"
network = "main"
```

```toml [survival.toml]
domains = ["survival.mc.example.com"]
addresses = ["10.0.1.10:25565"]
proxy_mode = "client_only"
max_players = 100
network = "main"

[timeouts]
connect = "3s"
read = "30s"
write = "30s"

[motd.online]
text = "§aSurvival §7— §fWelcome!"
```

```toml [creative.toml]
domains = ["creative.mc.example.com"]
addresses = ["127.0.0.1:25566"]
proxy_mode = "client_only"

[server_manager]
type = "local"
command = "java"
working_dir = "/opt/minecraft/creative"
args = ["-Xmx4G", "-jar", "server.jar", "nogui"]
ready_pattern = 'For help, type "help"'
shutdown_timeout = "30s"
shutdown_after = "15m"
```

:::
