---
title: Permissions
description: Control who can use proxy commands with permission nodes, answered by the built-in provider or by a permission plugin.
outline: [2, 3]
---

# Permissions

Every proxy command, and every plugin command with a permission, is guarded by a **permission node**, a dotted name such as `infrarust.command.kick` or `demo.use`. One **permission provider** decides which nodes each player holds. The built-in provider uses the `[permissions]` section below: admins hold every node, and `player_commands` opens chosen `/ir` subcommands to everyone. A permission plugin can replace it entirely.

Players have no proxy command access by default. The proxy is invisible to them unless you open specific commands.

| Who | Access with the built-in provider |
|-----|------------------|
| Player | Only the `/ir` subcommands listed in `player_commands`, and plugin nodes whose default is `true` |
| Admin | Every node, so every proxy and plugin command |

## Configuration

Add a `[permissions]` section to your `infrarust.toml`:

```toml [infrarust.toml]
[permissions]
provider = "builtin"
admins = [
    "069a79f4-44e9-4726-a5be-fca90e38aaf5",
    "Shadowner",
]
player_commands = ["help", "version", "list", "server", "find"]
trust_offline_admins = false
```

The section is read at startup. Changing it needs a restart; use the console's `op` and `deop` to change admins at runtime.

### `provider`

| Type | Default |
|------|---------|
| `String` | `"builtin"` |

Who answers permission questions:

- `"builtin"`: the proxy itself, from `admins` and `player_commands`.
- Any other value: the id of the plugin that provides permissions, for example `"luckperms"`. That plugin must be loaded and hold the `permission-provider` capability (compiled-in plugins hold it by default). Only that plugin may register a provider; others are refused with a warning in the log.

With a plugin provider, `admins` and `trust_offline_admins` are not used, and `op`, `deop` and `ops` refuse to run: manage admins in the plugin. `player_commands` still applies, see [Nodes and defaults](#nodes-and-defaults).

::: warning Missing provider
If `provider` names a plugin that never registers a provider (it failed to load, is disabled, or the id is misspelled), the proxy logs an error at startup and players only get the node defaults: `/ir` subcommands in `player_commands` still work, nobody is an admin, and admin-only commands are denied. Players can still join. The console keeps every permission.
:::

### `admins`

| Type | Default |
|------|---------|
| `Vec<String>` | `[]` (empty) |

A list of admin identifiers. Each entry can be a Mojang UUID (with dashes) or a username. Usernames are resolved to UUIDs via the Mojang API at startup.

A player is recognized as admin when both conditions are met:

1. Their UUID matches an entry in `admins`.
2. They authenticated in online mode (`client_only`), or `trust_offline_admins` is `true`.

An admin holds `infrarust.admin` and every other node.

::: tip
Prefer UUIDs over usernames. Username resolution requires an API call to Mojang at startup and will fail if the API is unreachable.
:::

### `trust_offline_admins`

| Type | Default |
|------|---------|
| `bool` | `false` |

Whether a player who did not authenticate with Mojang can be an admin. This covers `offline` servers and the forwarding modes (`passthrough`, `zero_copy`, `server_only`), where the proxy never checks who the player is.

::: danger
An offline player's UUID is derived from the name the client sends. With `trust_offline_admins = true`, anyone who connects with an admin's name gets admin rights. Only turn it on when a trusted layer in front of Infrarust, or the backend, authenticates players.
:::

### `player_commands`

| Type | Default |
|------|---------|
| `Vec<String>` | `[]` (empty) |

Subcommands of `/ir` that all players can use. When empty, non-admin players cannot see or use `/ir` at all. `"*"` opens every subcommand that may be opened.

Subcommands you can open to players:

| Command | Description |
|---------|-------------|
| `help` | Show help for proxy commands |
| `version` | Show proxy version and status |
| `list` | List all servers |
| `server` | Show or switch current server |
| `find` | Find which server a player is on |

An entry also grants `infrarust.command.<entry>` to every player through the built-in provider, so a plugin command guarded by `infrarust.command.<name>` can be opened the same way.

### Protected commands

These commands are admin-only. Adding them to `player_commands` has no effect (a warning is logged at startup):

| Command | Description |
|---------|-------------|
| `kick` | Kick a player from the proxy |
| `send` | Send a player to a server |
| `broadcast` | Broadcast a message to all players |
| `reload` | Configuration reload |
| `plugin` | Run a plugin command by namespace |
| `plugins` | List loaded plugins |

A permission plugin can still grant one of them to a group, for example `infrarust.command.kick` to moderators.

## Nodes and defaults

A node the provider has no answer for falls back to the default it was registered with: `true`, `false`, or `admin` (granted to players who hold `infrarust.admin`). A node nobody registered is denied.

| Node | Default |
|------|---------|
| `infrarust.admin` | `false`; the built-in provider grants it to admins |
| `infrarust.command.<name>` | `true` when `<name>` is in `player_commands` and may be opened, `admin` otherwise |
| Plugin nodes | Chosen by the plugin that registers them |

Because `player_commands` sets defaults, it keeps working with a plugin provider: players can use those subcommands unless the plugin explicitly denies the node.

### Wildcards

The built-in provider matches `a.b.*` against `a.b.c` and anything deeper, but not against `a.b` itself. `*` matches every node. The most specific entry wins, so an exact node beats `a.b.*`, which beats `a.*`, which beats `*`. Admins behave as if they held `*`. A permission plugin may use its own matching rule; see its documentation.

## Tab-completion filtering

The Brigadier command tree sent to each client only includes commands that player can use. Players with no permissions don't see `/ir` in tab-complete at all. Players with `player_commands = ["help", "list"]` only see those two. Admins see everything.

When a player's permissions change while they are online (`op`, `deop`, or a permission plugin refreshing them), that player gets a new command tree right away.

## Console commands

The console holds every permission with the built-in provider. Its own commands (`op`, `kick`, `ban`, `stop`...) never check permissions. With a plugin provider, plugin commands run from the console are checked against what the plugin grants the console. Use the console to manage admins at runtime:

| Command | Description |
|---------|-------------|
| `op <username>` | Grant admin to a player |
| `deop <username>` | Revoke admin from a player |
| `ops` | List current admins |

These commands only work with the built-in provider.

::: warning Persistence
Changes from `op` and `deop` take effect immediately but don't survive a restart. To persist them, add the player's UUID to `[permissions].admins` in `infrarust.toml`.
:::

### The op command

When you run `op <username>`:

1. If the player is online and authenticated in online mode, their Mojang UUID is used directly.
2. If the player is not online, the username is resolved via the Mojang API.
3. The UUID is added to the admin set. An online player gets admin access and a new command tree immediately, no reconnect needed.

If the player is connected in offline mode, `op` is rejected unless `trust_offline_admins` is `true`.

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

### An offline network behind an authenticating front

```toml
[permissions]
admins = ["5f41c03d-3aa8-37a6-995b-55787872865f"]
trust_offline_admins = true
```

The UUID is the offline UUID of the name `Shadowner`, the one an offline server gives that player. Only safe when players cannot reach Infrarust without being authenticated first.

### A permission plugin

```toml
[permissions]
provider = "luckperms"
player_commands = ["help", "list"]
```

The `luckperms` plugin decides every node. `help` and `list` stay open to players the plugin has no rule for.

### No config

Without a `[permissions]` section, no player has proxy command access. Use the console to bootstrap your first admin:

```
> op Shadowner
Opped Shadowner (UUID: 069a79f4-44e9-4726-a5be-fca90e38aaf5).
Change is effective until restart. Add UUID to [permissions].admins in infrarust.toml to persist.
```

## Plugin integration

See [Permissions API](../../plugins/dev/permissions.md) for how plugins register nodes, check permissions, refresh a player, and become the permission provider, and [Commands](../../plugins/dev/commands.md#permission-nodes) for guarding a command with a node.
