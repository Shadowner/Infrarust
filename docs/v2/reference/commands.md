---
title: In-game command reference
description: All subcommands for /infrarust (alias /ir), including usage, arguments, and required permission level.
outline: [2, 3]
---

# In-game command reference

The proxy registers one in-game command, `/infrarust`, with the alias `/ir`. Both names work identically. Tab-completion is permission-aware: subcommands the caller cannot use are not suggested.

There are exactly two permission levels, `Player` and `Admin` (`Player < Admin`). Who counts as an admin is configured under `[permissions].admins` in `infrarust.toml`. Plugins may also supply a custom permission checker via `PermissionsSetupEvent`.

The table below gives a quick overview. Details for each subcommand follow.

| Subcommand | Usage | Level |
|---|---|---|
| `help` | `/ir help [command]` | Player |
| `version` | `/ir version` | Player |
| `list` | `/ir list` | Player |
| `find` | `/ir find <player>` | Player |
| `server` | `/ir server [name]` | Player |
| `send` | `/ir send <player> <server>` | Admin |
| `broadcast` | `/ir broadcast <message> [--server <name>]` | Admin |
| `kick` | `/ir kick <player> [reason]` | Admin |
| `plugins` | `/ir plugins` | Admin |
| `plugin` | `/ir plugin <plugin_id> [command] [args...]` | Admin |
| `reload` | `/ir reload` | Admin |

---

## Player-level subcommands

These are available to any player whose permission level is `Player` or higher.

### help

```
/ir help [command]
```

With no argument, lists all subcommands visible to the caller (admin-only ones are hidden for non-admins). With a subcommand name, prints the usage string and description for that specific subcommand.

### version

```
/ir version
```

Prints the proxy version, the current online player count, and the proxy uptime. The uptime drops leading zero units, so it shows `Xd Xh Xm`, `Xh Xm`, or `Xm` depending on how long the proxy has run.

### list

```
/ir list
```

Lists all configured backend servers with a player count for each. If the server manager is active, a state indicator is shown (online `●`, sleeping `○`, crashed `✗`). Servers not tracked by the server manager show a `-`.

### find

```
/ir find <player>
```

Reports which backend server a named player is currently on. Tab-completes online player names. Returns an error if the player is not online.

### server

```
/ir server [name]
```

With no argument, shows the server you are currently connected to. With a server name, switches you to that backend. This command requires an intercepted proxy mode (`client_only` or `offline`) on the target connection; it will fail on forwarding-only modes that do not support server switching. Tab-completes configured server IDs.

---

## Admin-level subcommands

These require `PermissionLevel::Admin`. Players at the `Player` level will receive "You don't have permission." and the subcommands will not appear in tab-completion results.

### send

```
/ir send <player> <server>
```

Transfers the named online player to the named backend server. Both arguments are required. Errors if either the player is not online or the server ID is not in the configuration. Tab-completes player names for the first argument and server IDs for the second.

### broadcast

```
/ir broadcast <message> [--server <name>]
```

Sends a message to all connected players. Pass `--server <name>` to target only the players on one backend. The message supports legacy Minecraft color codes (e.g. `&c` for red). The confirmation reply indicates how many players received the message and, if `--server` was used, which server was targeted.

### kick

```
/ir kick <player> [reason]
```

Disconnects the named player from the proxy. The reason is optional; if omitted, the disconnect message defaults to `Kicked by proxy`. The reason may be multiple words. Tab-completes online player names.

### plugins

```
/ir plugins
```

Lists all loaded WASM plugins: name, version, and description for each. Prints "No plugins loaded." if the plugin registry is empty.

### plugin

```
/ir plugin <plugin_id> [command] [args...]
```

Has three forms depending on argument count:

- No arguments: same output as `plugins`, listing all loaded plugins.
- One argument (`plugin_id`): lists the commands registered by that plugin.
- Two or more arguments (`plugin_id command [args...]`): dispatches to the named plugin command, forwarding any trailing arguments.

Tab-completes plugin IDs at position one and plugin command names at position two.

### reload

```
/ir reload
```

Informs you that configuration reloads automatically when config files change on disk. No manual action is required. This subcommand exists as a discoverable entry point; it does not trigger a reload itself.

---

## Permission configuration

Admins are declared in `infrarust.toml`:

```toml
[permissions]
admins = ["PlayerOne", "PlayerTwo"]
```

Names in `admins` may be either Minecraft usernames or UUIDs. Usernames are resolved to UUIDs through the Mojang API at load time, and the check itself always compares UUIDs.

Plugins can provide a custom `PermissionChecker` that overrides per-player permission levels at runtime. They do this by handling the `PermissionsSetupEvent` and returning a checker, which is the integration point for external systems such as a permissions database. See [permissions](../configuration/security/permissions) for the configuration details.
