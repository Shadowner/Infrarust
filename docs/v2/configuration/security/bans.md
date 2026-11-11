---
title: Bans
description: Block players by IP address, IP range, username, or UUID with permanent or temporary bans, kept by the built-in store or by a ban plugin.
---

# Bans

Infrarust blocks players by IP address, IP range (CIDR), username, or Mojang UUID. Bans can be permanent or temporary, and they take effect immediately: connected players are kicked the moment you issue the ban.

Bans come from one **provider**. The built-in provider keeps them in a JSON file. A ban plugin can take over instead, and then every check and every ban command goes through that plugin.

## Configuration

The ban system is configured under the `ban` key in your proxy config:

::: code-group

```toml [infrarust.toml]
[ban]
provider = "builtin"
file = "bans.json"
purge_interval = "5m"
enable_audit_log = true
```

```yaml [infrarust.yml]
ban:
  provider: builtin
  file: bans.json
  purge_interval: 5m
  enable_audit_log: true
```

:::

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `provider` | string | `builtin` | Who decides who is banned: `builtin`, `none`, or a plugin id. See [Choosing a provider](#choosing-a-provider). |
| `file` | path | `bans.json` | Path to the JSON file where the built-in provider stores bans |
| `purge_interval` | duration | `5m` | How often the built-in provider cleans up expired bans |
| `enable_audit_log` | bool | `true` | Track ban/unban operations in the ban file |
| `check_timeout` | duration | `5s` | How long a login or ping waits for the provider's answer. Must be greater than zero. |

All options are optional. The defaults above apply if you omit the `[ban]` section entirely. `file`, `purge_interval` and `enable_audit_log` only matter with the built-in provider.

## Choosing a provider

| `provider` | Effect |
|------------|--------|
| `builtin` | Bans live in `file`. This is the default. |
| `none` | No ban checks: everyone may join. Ban commands, the admin API and plugins get a "bans are disabled" error. The ban file is not read. |
| any other value | The plugin with that id provides bans. The ban file is not read; the plugin keeps bans wherever it wants. |

With a plugin provider, the plugin has to register itself when it is enabled. If it does not (it failed to load, it is disabled, or the id is misspelled), Infrarust logs an error at startup and **refuses every login** with "Your ban status cannot be checked right now. Please try again later." until it does. Server list pings are still answered. The same applies while the plugin reports that its storage is unreachable, and to a check that gets no answer within `check_timeout`.

This is deliberate: you told the proxy that this plugin decides who is banned, and letting everyone in without it would quietly lift every ban it holds. If you want no ban checks, say so with `provider = "none"`.

Only the configured plugin can provide bans. Another plugin that tries is refused and a warning is logged. Plugin authors: see [Bans for plugin developers](../../plugins/dev/bans).

## Console commands

Manage bans from the Infrarust console. Duration values use the `humantime` format: `30s`, `10m`, `2h`, `7d`, `1y`, or `permanent` (also `perm`).

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

When some servers use a forwarding proxy mode (`passthrough`, `zero_copy`, `server_only`), the command also prints a warning that names them: the proxy does not authenticate players there, so the ban is checked against a name it has not verified. The warning suggests an IP ban or a ban on the backend. Servers on `offline` get a warning of their own: nobody authenticates players there, so the ban only blocks the name or UUID a client presents, and the player can get back in under another name. That warning suggests an IP ban. See [what each proxy mode can enforce](#what-each-proxy-mode-can-enforce).

### ban-ip

Ban an IP address or a CIDR range. All players currently connected from a matching address are disconnected. Also available as `banip`.

```
ban-ip <ip|cidr> [duration] [reason...]
```

```
ban-ip 192.168.1.100 24h suspicious activity
ban-ip 10.0.0.50 permanent
ban-ip 203.0.113.0/24 7d botnet
ban-ip 2001:db8:abcd::/48 permanent
```

Addresses are matched on the player's real IP, the one from the PROXY protocol header when [`receive_proxy_protocol`](./proxy-protocol) is on. An IPv4 ban also matches a client that reaches a dual-stack listener as `::ffff:a.b.c.d`.

### unban

Remove a username ban. Also available as `pardon`.

```
unban <player>
```

### unban-ip

Remove an IP or range ban. The argument must be written the way the ban was issued. Also available as `unbanip` or `pardonip`.

```
unban-ip <ip|cidr>
```

### banlist

List all active bans in a table showing id, target, type, reason, source, and remaining time. Also available as `bans`.

```
banlist
```

### baninfo

Show full details of a specific ban. The argument is auto-detected as an IP, CIDR range, UUID, or username.

```
baninfo <player|ip|cidr|uuid>
```

```
baninfo Griefer123
baninfo 192.168.1.100
baninfo 550e8400-e29b-41d4-a716-446655440000
```

## How bans are checked

Infrarust asks the provider at three points:

1. **Server list ping.** A ping from a banned address or range is closed without an answer, so banned addresses do not see the MOTD. Legacy pings are checked too.
2. **Login start**, after the client sends its username and before authentication. The built-in provider checks the IP, IP ranges and the username (case-insensitive).
3. **After authentication**, once the final profile is known (after `GameProfileRequestEvent`, in every proxy mode). The built-in provider now also checks the UUID.

A connection that a plugin takes over with a [connection handoff](../../plugins/wasm/connection-handoff) is checked before it is relayed, on the address and, for a claimed login, on the username from the Login Start, so a banned name is not handed off. UUID bans do not apply to a handoff.

A refused login is disconnected in the login state with the ban message. With the built-in provider the message shows the ban reason and, for temporary bans, the remaining time; a plugin provider shows its own message.

### What each proxy mode can enforce

A ban holds only as far as the proxy knows who the player is. Only `client_only` verifies the name and UUID with the session server. In the other modes the proxy checks what the client claims:

| Ban type | `client_only` | `offline` | `passthrough`, `zero_copy`, `server_only` |
|----------|---------------|-----------|-------------------------------------------|
| IP, IP range | Enforced | Enforced | Enforced |
| Username | Enforced on the verified name | Checked against the claimed name | Checked against the claimed name |
| UUID | Enforced on the Mojang UUID | Checked against the UUID derived from the name | Checked against the UUID derived from the name |

In `offline` and the forwarding modes, the UUID is the name-based offline UUID, or the one the client sends with [`[auth] offline_uuid = "client"`](../global#authentication). A ban on a player's Mojang UUID does not match there with the default setting.

In the forwarding modes every check runs before authentication: the backend authenticates the player after the proxy has let the connection through, and the proxy never sees the result. For those servers, add an IP ban or ban the player on the backend as well.

In `offline` nobody verifies the name, so a banned player can come back under another one. For those servers, add an IP ban.

## Storage

Bans are stored in a JSON file (default `bans.json`) next to your proxy config. The file contains two arrays: `bans` (active ban entries) and `audit_log` (history of ban/unban actions).

A ban entry looks like this:

```json
{
  "id": "12",
  "target": { "type": "username", "value": "Griefer123" },
  "reason": "griefing the spawn area",
  "expires_at": 1711324800,
  "created_at": 1710720000,
  "source": { "type": "console" }
}
```

Every ban gets an `id` that is never reused, even across restarts (the file keeps the last one in `next_id`). Range bans use `{ "type": "ip_range", "value": "203.0.113.0/24" }`. Permanent bans have `expires_at` set to `null`. The `source` field records who issued the ban: `console`, `web_api` (the admin API), `plugin` with the plugin id as `value`, `player`, or `system`.

Files written by earlier versions still load: entries without an `id` get one, and a plain string `source` is read as before (`"console"` is the console, `"plugin"` becomes a plugin source named `plugin`).

::: tip
You don't need to create `bans.json` manually. Infrarust creates it the first time you issue a ban. If the file doesn't exist at startup, or is empty, the proxy starts with an empty ban list.
:::

::: warning
If the file has content that cannot be parsed, Infrarust refuses to start with `ban file <path> is corrupt`, because starting without it would lift every ban. Repair the file, or move it away to start with no bans.
:::

Writes are crash-safe: the proxy writes to a temporary file first, then atomically renames it over the existing file. The audit log is capped at 10,000 entries to keep the file from growing without bound.

## Plugin API

Plugins manage bans through `PluginContext::ban_service()`, whichever provider is active, and a ban plugin can become the provider. Each ban records its source, and every ban and unban is announced with `BanIssuedEvent` and `BanRevokedEvent`. See [Bans for plugin developers](../../plugins/dev/bans).
