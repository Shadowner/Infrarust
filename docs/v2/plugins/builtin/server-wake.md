---
title: Server Wake Plugin
description: Hold players in limbo while Minecraft servers start up, with support for Local, Pterodactyl, and Crafty providers
---

# Server Wake Plugin

The server wake plugin holds players in a limbo state while their target server starts up. When a player connects to a sleeping server, the plugin sends a start signal through the configured provider, shows an animated title screen, and releases the player once the server is online.

## How it works

1. A player connects (or gets kicked from a server that's restarting).
2. The plugin checks the target server's state through the server manager.
3. If the server is sleeping, crashed, or offline, the plugin tells the provider to start it.
4. The player enters limbo and sees an animated "Server Starting" title with rotating dots.
5. An action bar shows how many players are waiting for the same server.
6. When the server comes online, all waiting players see "Server Ready!" and get forwarded.
7. If the server crashes or the timeout expires, waiting players are kicked with an error message.

## Server manager configuration

The server manager is configured per-server in your server config file, not in the plugin config. You pick one of three provider types: `local`, `pterodactyl`, or `crafty`.

### Local provider

Launches a Java process directly on the same machine. The provider watches stdout for a pattern that indicates the server is ready.

```toml
[server_manager]
type = "local"
command = "java"
working_dir = "/opt/minecraft/survival"
args = ["-Xmx4G", "-jar", "server.jar", "nogui"]
ready_pattern = 'For help, type "help"'
start_timeout = "60s"
shutdown_timeout = "30s"
shutdown_after = "30m"
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `command` | string | required | Executable to run |
| `working_dir` | path | required | Working directory for the process |
| `args` | string array | `[]` | Command-line arguments |
| `ready_pattern` | string | `For help, type "help"` | Stdout pattern that signals the server is ready |
| `start_timeout` | duration | `60s` | How long to wait for the server to become ready |
| `shutdown_timeout` | duration | `30s` | How long to wait for graceful shutdown before killing |
| `shutdown_after` | duration | none | Shut down automatically after this idle duration. Disabled if omitted |

The local provider sends `stop\n` to the process's stdin for graceful shutdown. If the process doesn't exit within `shutdown_timeout`, it gets killed.

### Pterodactyl provider

Controls servers through the Pterodactyl panel API. You need a client API key with power permissions for the target server.

```toml
[server_manager]
type = "pterodactyl"
api_url = "https://panel.example.com"
api_key = "ptlc_xxxxxxxxxxxxxxxxxxxx"
server_id = "a1b2c3d4"
start_timeout = "60s"
poll_interval = "5s"
shutdown_after = "30m"
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `api_url` | string | required | Pterodactyl panel URL |
| `api_key` | string | required | Client API key |
| `server_id` | string | required | Server identifier from the panel |
| `start_timeout` | duration | `60s` | How long to wait for the server to start |
| `poll_interval` | duration | `5s` | How often to poll the API for state changes |
| `shutdown_after` | duration | none | Shut down automatically after this idle duration |

The provider maps Pterodactyl states: `running` to Online, `starting` to Starting, `stopping` to Stopping, `offline` to Stopped.

### Crafty Controller provider

Controls servers through the Crafty Controller v2 API.

```toml
[server_manager]
type = "crafty"
api_url = "https://crafty.example.com"
api_key = "your-crafty-api-key"
server_id = "1"
start_timeout = "60s"
poll_interval = "5s"
shutdown_after = "30m"
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `api_url` | string | required | Crafty Controller URL |
| `api_key` | string | required | API token |
| `server_id` | string | required | Server ID in Crafty |
| `start_timeout` | duration | `60s` | How long to wait for the server to start |
| `poll_interval` | duration | `5s` | How often to poll the API for state changes |
| `shutdown_after` | duration | none | Shut down automatically after this idle duration |

## Plugin configuration

The plugin stores its own config in `plugins/server_wake/config.toml`. On first run, it creates the file with defaults. This config controls the limbo experience (titles, messages, timeouts), not the server management itself.

### Timing

```toml
[timing]
start_timeout_seconds = 180
title_refresh_interval_seconds = 3
show_waiting_count = true
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `start_timeout_seconds` | integer | `180` | Seconds before a waiting player gets kicked. Set to `0` to disable |
| `title_refresh_interval_seconds` | integer | `3` | How often the animated title updates (in seconds) |
| `show_waiting_count` | bool | `true` | Show an action bar with the number of players waiting |

::: tip
The plugin's `start_timeout_seconds` (180s default) is separate from the provider's `start_timeout` (60s default). The provider timeout controls how long the server manager waits internally. The plugin timeout controls how long a player sits in limbo before getting kicked.
:::

### Messages

Every message supports Minecraft color codes (`&a`, `&c`, `&7`, etc.) and placeholders: `{server}` for the server name, `{dots}` for the animated dots, and `{count}` for the waiting player count.

```toml
[messages]
starting_title = "&eServer Starting"
starting_subtitle = "&7Please wait&f{dots}"
stopping_title = "&eServer Restarting"
stopping_subtitle = "&7Waiting for shutdown&f{dots}"
ready_title = "&aServer Ready!"
ready_subtitle = "&7Connecting you now..."
failed_kick = "&cThe server failed to start. Please try again later."
timeout_kick = "&cThe server took too long to start. Please try again."
waiting_action_bar = "&7{count} player(s) waiting for &e{server}"
```

The `starting_*` messages show while a server is booting. The `stopping_*` messages show if a player connects while the server is shutting down (the plugin waits for it to stop, then start again). The `ready_*` title flashes briefly before the player gets forwarded.

## MOTD during startup

While a server is managed by the server manager, the server list ping shows a state-specific MOTD instead of the backend's actual response. You configure these in the server config file under `[motd]`:

```toml
[motd.sleeping]
text = "§7Server sleeping — §aConnect to wake up!"

[motd.starting]
text = "§eServer is starting..."

[motd.stopping]
text = "§6Server is stopping..."

[motd.crashed]
text = "§cServer unavailable"
```

Each MOTD entry accepts these fields:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `text` | string | varies by state | MOTD text shown in the server list |
| `favicon` | string | none | Path to a PNG, base64 string, or URL |
| `version_name` | string | none | Custom version string in the server list |
| `max_players` | integer | none | Max player count shown in the server list |

If you don't configure a custom MOTD for a state, Infrarust uses built-in defaults: "Server sleeping — Connect to wake up!" for sleeping, "Server is starting..." for starting, "Server is stopping..." for stopping, and "Server unavailable" for crashed.

## Auto-shutdown

All three providers support `shutdown_after`. When set, the server manager monitors the player count. If no players are connected for the specified duration, the server gets stopped automatically. Omit the field or leave it unset to disable auto-shutdown.

The monitor polls the provider at `poll_interval` intervals during state transitions (starting, stopping) and at 6x the poll interval when the server is stable (running or stopped).

## Full example

A server config file with Pterodactyl management and custom MOTD:

```toml
addresses = ["survival.example.com"]
proxy_mode = "client_only"

[proxy_to]
address = "10.0.0.5:25565"

[server_manager]
type = "pterodactyl"
api_url = "https://panel.example.com"
api_key = "ptlc_xxxxxxxxxxxxxxxxxxxx"
server_id = "a1b2c3d4"
start_timeout = "120s"
shutdown_after = "15m"
poll_interval = "3s"

[motd.sleeping]
text = "§7Survival is sleeping — §aJoin to wake it up!"
favicon = "sleeping_icon.png"

[motd.starting]
text = "§eSurvival is booting up, hang tight..."
```
