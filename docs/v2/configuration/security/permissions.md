---
title: Permissions
description: Control who can use proxy commands with a two-level permission system.
outline: [2, 3]
---

# Permissions

Infrarust has two permission levels: Player and Admin. Players have no command access by default. The proxy is invisible to them unless you open specific commands in the config.

| Level  | Access |
|--------|--------|
| Player | No command access unless opened via `player_commands` |
| Admin  | Full access to all proxy commands |

## Configuration

Add a `[permissions]` section to your `infrarust.toml`:

```toml [infrarust.toml]
[permissions]
admins = [
    "069a79f4-44e9-4726-a5be-fca90e38aaf5",
    "Shadowner",
]

player_commands = ["help", "version", "list", "server", "find"]
```

### `admins`

| Type | Default |
|------|---------|
| `Vec<String>` | `[]` (empty) |

A list of admin identifiers. Each entry can be a Mojang UUID (with dashes) or a username. Usernames are resolved to UUIDs via the Mojang API at startup.

A player is recognized as admin when both conditions are met:

1. Their UUID matches an entry in `admins`.
2. They authenticated in online mode (`client_only` or `ForceOnline`).

::: warning
Offline-mode players can never be admin, even if their UUID appears in the list. This prevents UUID spoofing.
:::

::: tip
Prefer UUIDs over usernames. Username resolution requires an API call to Mojang at startup and will fail if the API is unreachable.
:::

### `player_commands`

| Type | Default |
|------|---------|
| `Vec<String>` | `[]` (empty) |

Subcommands of `/ir` that all players can use. When empty, non-admin players cannot see or use `/ir` at all.

Subcommands you can open to players:

| Command | Description |
|---------|-------------|
| `help` | Show help for proxy commands |
| `version` | Show proxy version and status |
| `list` | List all servers |
| `server` | Show or switch current server |
| `find` | Find which server a player is on |

### Protected commands

These commands are always admin-only. Adding them to `player_commands` has no effect (a warning is logged at startup):

| Command | Description |
|---------|-------------|
| `kick` | Kick a player from the proxy |
| `send` | Send a player to a server |
| `broadcast` | Broadcast a message to all players |
| `reload` | Configuration reload |
| `plugin` | Run a plugin command by namespace |

## Tab-completion filtering

The Brigadier command tree sent to each client only includes commands that player can use. Players with no permissions don't see `/ir` in tab-complete at all. Players with `player_commands = ["help", "list"]` only see those two. Admins see everything.

## Console commands

The admin console bypasses all permission checks. Use it to manage admins at runtime:

| Command | Description |
|---------|-------------|
| `op <username>` | Grant admin to a player |
| `deop <username>` | Revoke admin from a player |
| `ops` | List current admins |

::: warning Persistence
Changes from `op` and `deop` take effect immediately but don't survive a restart. To persist them, add the player's UUID to `[permissions].admins` in `infrarust.toml`.
:::

### The op command

When you run `op <username>`:

1. If the player is online and authenticated in online mode, their Mojang UUID is used directly.
2. If the player is not online, the username is resolved via the Mojang API.
3. The UUID is added to the admin set. Online players get admin access immediately, no reconnect needed.

If the player is connected in offline mode, `op` is rejected.

## Examples

### Admin only

```toml
[permissions]
admins = ["069a79f4-44e9-4726-a5be-fca90e38aaf5"]
```

Only this admin can use proxy commands. Everyone else sees nothing.

### Open some commands to players

```toml
[permissions]
admins = ["Shadowner"]
player_commands = ["help", "version", "list", "server"]
```

All players can list servers and switch between them. Only `Shadowner` can kick, send, broadcast, or manage plugins.

### No config

Without a `[permissions]` section, no player has proxy command access. Use the console to bootstrap your first admin:

```
> op Shadowner
Opped Shadowner (UUID: 069a79f4-44e9-4726-a5be-fca90e38aaf5).
Change is effective until restart. Add UUID to [permissions].admins in infrarust.toml to persist.
```

## Plugin integration

See the [Commands](../../plugins/dev/commands.md#permission-checks) page for how plugins check permissions and how to provide a custom permission checker via `PermissionsSetupEvent`.
