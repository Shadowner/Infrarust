---
title: Bans
description: Block players by IP address, username, or UUID with permanent or temporary bans.
---

# Bans

Infrarust has a built-in ban system that blocks players by IP address, username, or Mojang UUID. Bans can be permanent or temporary, and they take effect immediately: connected players are kicked the moment you issue the ban.

## Configuration

The ban system is configured under the `ban` key in your proxy config:

::: code-group

```toml [infrarust.toml]
[ban]
file = "bans.json"
purge_interval = "5m"
enable_audit_log = true
```

```yaml [infrarust.yml]
ban:
  file: bans.json
  purge_interval: 5m
  enable_audit_log: true
```

:::

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `file` | path | `bans.json` | Path to the JSON file where bans are stored |
| `purge_interval` | duration | `5m` | How often expired bans are cleaned up |
| `enable_audit_log` | bool | `true` | Track ban/unban operations in the ban file |

All three options are optional. The defaults above apply if you omit the `[ban]` section entirely.

## Console Commands

Manage bans from the Infrarust console. Duration values use the `humantime` format: `30s`, `10m`, `2h`, `7d`, `1y`, or `permanent`.

### ban

Ban a player by username. If the player is currently connected, they are kicked immediately.

```
ban <player> [duration] [reason...]
```

```
ban Griefer123 7d griefing the spawn area
ban NotWelcome permanent
ban TempBan 2h
```

### ban-ip

Ban an IP address. All players currently connected from that IP are disconnected. Also available as `banip`.

```
ban-ip <ip> [duration] [reason...]
```

```
ban-ip 192.168.1.100 24h suspicious activity
ban-ip 10.0.0.50 permanent
```

### unban

Remove a username ban. Also available as `pardon`.

```
unban <player>
```

### unban-ip

Remove an IP ban. Also available as `unbanip` or `pardonip`.

```
unban-ip <ip>
```

### banlist

List all active bans in a table showing target, type, reason, source, and remaining time. Also available as `bans`.

```
banlist
```

### baninfo

Show full details of a specific ban. The argument is auto-detected as an IP, UUID, or username.

```
baninfo <player|ip|uuid>
```

```
baninfo Griefer123
baninfo 192.168.1.100
baninfo 550e8400-e29b-41d4-a716-446655440000
```

## How Bans Are Checked

Infrarust checks bans at two points in the connection pipeline:

1. **IP check** runs before the handshake. If the connecting IP is banned, the connection is dropped immediately with no server response.
2. **Full check** runs during login, after the client sends its username. This checks the player's IP, username (case-insensitive), and UUID against the ban list.

Banned players see a kick message with the ban reason and, for temporary bans, the remaining time.

## Storage

Bans are stored in a JSON file (default `bans.json`) next to your proxy config. The file contains two arrays: `bans` (active ban entries) and `audit_log` (history of ban/unban actions).

A ban entry looks like this:

```json
{
  "target": { "type": "username", "value": "Griefer123" },
  "reason": "griefing the spawn area",
  "expires_at": 1711324800,
  "created_at": 1710720000,
  "source": "console"
}
```

Permanent bans have `expires_at` set to `null`. The `source` field records who issued the ban: `"console"` for console commands, `"plugin"` for plugin-initiated bans.

::: tip
You don't need to create `bans.json` manually. Infrarust creates it the first time you issue a ban. If the file doesn't exist at startup, the proxy starts with an empty ban list.
:::

Writes are crash-safe: the proxy writes to a temporary file first, then atomically renames it over the existing file. The audit log is capped at 10,000 entries to keep the file from growing without bound.

## Plugin API

Plugins can manage bans through the `BanService` trait, available via `PluginContext::ban_service()`. The API provides five methods:

- `ban(target, reason, duration)` — add a ban (duration `None` = permanent)
- `unban(target)` — remove a ban, returns `true` if one was removed
- `is_banned(target)` — check if a target is banned, returns `true`/`false`
- `get_ban(target)` — get the full `BanEntry` if banned
- `get_all_bans()` — list all active bans

Ban targets are constructed with `BanTarget::Ip(addr)`, `BanTarget::Username(name)`, or `BanTarget::Uuid(uuid)`.
