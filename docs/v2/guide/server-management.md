---
title: Server management
description: Wake a backend server when a player connects and stop it when idle, using the local process, Pterodactyl, or Crafty Controller provider.
---

# Server management

Infrarust can start a backend server the moment a player connects to it, then shut it down after the last player leaves. This keeps resource usage low on networks where some servers sit idle for hours at a time.

The feature has two parts: the `[server_manager]` table in a server's config file selects a provider and sets timing parameters, while the `server-wake` plugin holds players in limbo while the backend boots.

## How it works

When a player connects to a managed server:

1. The server manager receives a wake request and calls the provider's start method.
2. The `server-wake` limbo handler suspends the player's session and shows an animated title screen.
3. The manager polls the provider until the server reports `Online`, then notifies all waiters.
4. The limbo handler releases each waiting player into the now-live backend.

If the server was already online the connection proceeds normally with no delay. If it crashes during startup, waiting players are kicked with a configurable message.

Auto-shutdown runs on the same polling loop. Once the last player disconnects, the manager starts a timer. When `shutdown_after` elapses with no players, it calls the provider's stop method.

## Server states

| State | Meaning |
|-------|---------|
| `Sleeping` | Stopped, can be woken on demand |
| `Starting` | Start request sent, waiting for ready signal |
| `Online` | Running and accepting connections |
| `Stopping` | Stop request sent |
| `Crashed` | Exited unexpectedly |
| `Unknown` | State could not be determined |

## Providers

Three providers are built in. The `type` field in `[server_manager]` selects which one.

### Local process

Spawns a process on the same host as the proxy. The process's stdout is scanned for a pattern that marks it ready. For vanilla and most modded servers the default pattern (`For help, type "help"`) works without any configuration.

The local provider sends the `stop` command on stdin for graceful shutdown, then waits up to `shutdown_timeout` before killing the process.

```toml
# servers/survival.toml
name = "survival"
domains = ["survival.mc.example.com"]
addresses = ["127.0.0.1:25565"]
proxy_mode = "client_only"
limbo_handlers = ["server_wake"]

[server_manager]
type = "local"
command = "java"
args = ["-Xmx4G", "-jar", "server.jar", "nogui"]
working_dir = "/opt/minecraft/survival"
shutdown_after = "10m"
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `command` | string | required | Executable to run |
| `working_dir` | string | required | Process working directory |
| `args` | array | `[]` | Arguments passed to the command |
| `ready_pattern` | string | `For help, type "help"` | Substring in stdout that signals the server is ready |
| `shutdown_timeout` | duration | `"30s"` | Time to wait for graceful shutdown before killing the process |
| `start_timeout` | duration | `"60s"` | Maximum wait for the ready pattern before declaring the start failed |
| `shutdown_after` | duration | none | Idle time after the last player leaves before auto-shutdown. Omit to disable |

### Pterodactyl

Sends power signals to a Pterodactyl panel via its client API. Infrarust polls the `/resources` endpoint for state transitions.

```toml
[server_manager]
type = "pterodactyl"
api_url = "https://panel.example.com"
api_key = "ptlc_xxxxxxxxxxxxx"
server_id = "abc12345"
shutdown_after = "15m"
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `api_url` | string | required | Panel URL (trailing slash is stripped automatically) |
| `api_key` | string | required | Client API key with power control permission |
| `server_id` | string | required | Server identifier shown in the panel URL |
| `start_timeout` | duration | `"60s"` | Maximum wait before declaring startup failed |
| `poll_interval` | duration | `"5s"` | How often to poll the panel API |
| `shutdown_after` | duration | none | Idle time before auto-shutdown. Omit to disable |

### Crafty Controller

Calls the Crafty Controller v2 API to start and stop servers. State is inferred from the `data.running` boolean in the stats endpoint.

```toml
[server_manager]
type = "crafty"
api_url = "https://crafty.example.com"
api_key = "your-api-key"
server_id = "your-server-uuid"
shutdown_after = "20m"
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `api_url` | string | required | Crafty panel URL |
| `api_key` | string | required | API key with server control permission |
| `server_id` | string | required | Crafty server UUID |
| `start_timeout` | duration | `"60s"` | Maximum wait before declaring startup failed |
| `poll_interval` | duration | `"5s"` | How often to poll the panel API |
| `shutdown_after` | duration | none | Idle time before auto-shutdown. Omit to disable |

::: info
Crafty does not expose a detailed state enum, only a running/stopped boolean. The provider maps `running = true` to `Online` and `running = false` to `Sleeping`. There is no `Starting` intermediate state from the API side, though Infrarust still tracks its own internal starting state.
:::

## The server-wake plugin

The `server-wake` plugin is what keeps players in limbo during startup. Without it, a player connecting to a sleeping server would get an immediate "connection refused" error. With it, they see a title screen and animated progress while the backend wakes up.

The plugin is a native (non-WASM) plugin that ships with Infrarust. To enable it, add `"server_wake"` to the `limbo_handlers` list on any server that has a `[server_manager]` configured.

```toml
# servers/survival.toml
limbo_handlers = ["server_wake"]
```

The server config must also use a proxy mode that supports limbo handlers (`client_only` or `offline`). Passthrough modes forward raw bytes and cannot intercept the connection to hold it.

The plugin's own config lives at `plugins/server_wake/config.toml` (created automatically on first run). You can adjust the messages and timing:

```toml
[timing]
start_timeout_seconds = 180
title_refresh_interval_seconds = 3
show_waiting_count = true

[messages]
starting_title = "&eServer Starting"
starting_subtitle = "&7Please wait&f{dots}"
ready_title = "&aServer Ready!"
ready_subtitle = "&7Connecting you now..."
failed_kick = "&cThe server failed to start. Please try again later."
timeout_kick = "&cThe server took too long to start. Please try again."
waiting_action_bar = "&7{count} player(s) waiting for &e{server}"
```

The `{dots}` placeholder cycles through `.`, `..`, `...` on each refresh tick. The `{server}` and `{count}` placeholders are available in most message fields.

::: warning
`start_timeout_seconds` in the plugin config is separate from `start_timeout` in `[server_manager]`. The plugin timeout kicks the player from limbo if they have been waiting too long. The manager timeout causes the start attempt itself to fail (and switches the server to `Crashed`). Set the plugin value higher than the manager value to avoid kicking players before the server has had a chance to fully start.
:::

## Proxy mode requirements

Limbo handlers require a proxy mode that parses packets. The local, Pterodactyl, and Crafty providers have no proxy mode restriction on their own, but the wake flow depends on the plugin holding the connection.

Compatible modes: `client_only`, `offline`.

Incompatible modes (the proxy cannot inspect packets): `passthrough`, `zero_copy`, `server_only`.

A server managed by `[server_manager]` but using a passthrough mode will still wake and stop automatically, but connecting players will see a connection error rather than a holding screen while the backend starts.

## MOTD while sleeping

You can customize what players see in the server list when the backend is sleeping or starting:

```toml
[motd.sleeping]
text = "§7Server sleeping, connect to wake it"
version_name = "Sleeping"
max_players = 0

[motd.starting]
text = "§eServer starting..."
version_name = "Starting"
max_players = 0
```

Players who only ping the server list (without connecting) will see these MOTDs. Players who actually connect get the limbo screen from the plugin.

## Full example

A survival server that sleeps when idle, wakes on connection, and uses the Pterodactyl panel:

```toml
# servers/survival.toml
name = "survival"
network = "main"
domains = ["survival.mc.example.com"]
addresses = ["10.0.1.10:25565"]
proxy_mode = "client_only"
limbo_handlers = ["server_wake"]

[motd.online]
text = "§aSurvival Online"

[motd.sleeping]
text = "§7Survival, connect to wake"
version_name = "Sleeping"
max_players = 0

[motd.starting]
text = "§eStarting up..."
version_name = "Starting"

[server_manager]
type = "pterodactyl"
api_url = "https://panel.example.com"
api_key = "ptlc_xxxxxxxxxxxxx"
server_id = "abc12345"
start_timeout = "120s"
shutdown_after = "15m"
```

## Custom providers

The `ServerProvider` trait is not sealed. You can implement your own provider for panels or launch systems not covered by the three built-in options and register it via `ServerManagerService::register_server`. See the `infrarust_server_manager` crate documentation for the trait definition.
