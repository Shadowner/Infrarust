---
title: Auth Plugin
description: Password-based authentication and premium auto-login for Minecraft proxies
---

# Auth Plugin

The auth plugin adds password-based authentication to offline-mode proxies. When a player connects, they're held in a limbo state and must `/register` (first visit) or `/login` (returning player) before reaching the backend server.

Players with a paid Minecraft account (premium) skip this. When premium auto-login is enabled, the plugin queries the Mojang API and triggers the Mojang encryption handshake (RSA + `hasJoined`). If the client proves ownership, it goes straight to the backend.

## How it works

### Offline (cracked) players

1. A player connects and enters the limbo state.
2. The plugin checks if an account exists for that username (case-insensitive).
3. New players see `/register <password> <confirm>`. Returning players see `/login <password>`.
4. A title and chat message remind the player what to do. Reminders repeat on an interval.
5. On success, the player leaves limbo and reaches the backend server.
6. On failure, the player gets feedback with remaining attempts. After too many failures, they're kicked.
7. If the player doesn't authenticate within the timeout, they're disconnected.

### Premium players

When `[premium] enabled = true`:

1. A player connects. Before entering limbo, a `PreLoginEvent` fires.
2. The plugin checks the Mojang API (`api.mojang.com`) to see if the username belongs to a paid account.
3. If premium, the proxy forces the Mojang encryption handshake (RSA key exchange + `sessionserver.mojang.com/session/minecraft/hasJoined`).
4. If the client proves it owns the account, the player's profile arrives with signed skin textures.
5. The auth handler sees the signed textures and returns `Accept`, skipping `/login` and `/register`.
6. If the client fails the encryption handshake (cracked client using a premium username), it gets disconnected. The plugin remembers the failure for the [second attempt](#cracked-players-using-premium-usernames-second-attempt).

::: tip
In `client_only` proxy mode, premium players are already Mojang-authenticated by the proxy core. The auth handler sees signed textures and skips the limbo with no API call. Cracked players get kicked on first connection but can join on their second attempt via the [remember mechanism](#cracked-players-using-premium-usernames-second-attempt).
:::

## Configuration

The plugin stores its config in `plugins/auth/config.toml`. On first run, it creates the file with defaults.

### Storage

```toml
[storage]
backend = "json"
path = "accounts.json"
auto_save_interval_seconds = 300
```

`backend` only supports `"json"` for now. Accounts are saved to `plugins/auth/accounts.json`. The plugin writes to disk on a periodic interval and on shutdown, using an atomic write (temp file + rename) to avoid corruption.

### Password hashing

```toml
[hashing]
argon2_memory_cost = 19456
argon2_time_cost = 2
argon2_parallelism = 1
migrate_legacy_hashes = true
```

Passwords are hashed with Argon2id. If `migrate_legacy_hashes` is `true`, bcrypt hashes from older setups are re-hashed to Argon2id on the next successful login.

The plugin generates a dummy hash at startup. When a player tries `/login` with a username that doesn't exist, the plugin runs a full Argon2 verify against the dummy hash. This prevents attackers from measuring response times to determine which usernames have accounts.

### Password policy

```toml
[password_policy]
min_length = 8
max_length = 128
blocked_passwords_file = "blocked_passwords.txt"
check_username = true
```

`blocked_passwords_file` points to a text file (one password per line) in the `plugins/auth/` directory. If the file doesn't exist, the blocklist is disabled. `check_username` rejects passwords that match the player's username.

### Security

```toml
[security]
max_login_attempts = 5
login_timeout_seconds = 60
title_reminder_interval_seconds = 5
```

After `max_login_attempts` wrong passwords, the player is kicked. Set `login_timeout_seconds` to `0` to disable the timeout. Set `title_reminder_interval_seconds` to `0` to disable periodic title reminders.

### Privacy

```toml
[privacy]
log_ip_masking = "last_two_octets"
```

Controls how player IPs appear in logs. Options:

| Value | IPv4 output | IPv6 output |
|-------|-------------|-------------|
| `last_two_octets` | `192.168.x.x` | `2001:db8:85a3:x:x:x:x:x` |
| `last_octet` | `192.168.1.x` | `2001:db8:85a3:1234:x:x:x:x` |
| `none` | `192.168.1.42` | Full address |

### Admin

```toml
[admin]
admin_usernames = []
```

Usernames listed here can use admin commands (`/forcelogin`, `/forceunregister`, `/forcechangepassword`). Usernames are case-insensitive. Players with the `auth.admin` permission also have admin access, regardless of this list.

### Premium auto-login {#premium}

```toml
[premium]
enabled = false
cache_ttl_seconds = 600
rate_limit_per_second = 1
rate_limit_action = "allow_offline"
premium_name_conflict_action = "kick"
allow_cracked_command = true
failed_auth_remember_seconds = 600
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | bool | `false` | Enable premium auto-login detection |
| `cache_ttl_seconds` | u64 | `600` | How long to cache Mojang API results (seconds) |
| `rate_limit_per_second` | u32 | `1` | Max Mojang API lookups per second |
| `rate_limit_action` | string | `"allow_offline"` | What to do when the rate limit is hit |
| `premium_name_conflict_action` | string | `"kick"` | What to do when a cracked client uses a premium name |
| `allow_cracked_command` | bool | `true` | Register `/cracked` and `/premium` commands |
| `failed_auth_remember_seconds` | u64 | `600` | How long to remember a failed premium auth (see below) |

`rate_limit_action` values:

| Value | Behavior |
|-------|----------|
| `"allow_offline"` (default) | Let the player through in offline mode. They see `/login` or `/register` as usual. |
| `"deny"` | Reject the connection. |

`premium_name_conflict_action` values:

| Value | Behavior |
|-------|----------|
| `"kick"` (default) | The cracked client fails the Mojang encryption handshake and gets disconnected. |
| `"allow_cracked"` | Fall back to the normal `/login` / `/register` flow. |

::: warning
The Mojang API has a rate limit of roughly 600 requests per 10 minutes. The `rate_limit_per_second` setting is a local governor to stay under that limit. On a high-traffic server with many first-time players, cached results handle most lookups, but a burst of new players will trigger the rate limit. The default `"allow_offline"` action ensures nobody is locked out.
:::

#### Premium messages

```toml
[premium.messages]
premium_login = "&aWelcome back, {username}! (Premium auto-login)"
premium_name_conflict = "&cThis username belongs to a premium account. Use the official Minecraft launcher."
cracked_enabled = "&aYou will now login as a cracked player. Reconnect to apply."
cracked_disabled = "&aYou will now login as a premium player. Reconnect to apply."
rate_limited = "&cThe server is busy. Please try again in a moment."
```

### Messages

Every message the plugin sends is configurable. Messages support Minecraft color codes (`&a`, `&c`, `&7`, etc.) and placeholders like `{username}`, `{attempts_left}`, `{max_attempts}`, `{min_length}`, `{max_length}`.

```toml
[messages]
login_title = "&6Authentication Required"
login_subtitle = "&7/login <password>"
login_success = "&aLogin successful!"
login_fail = "&cWrong password! &7({attempts_left}/{max_attempts} attempts left)"
login_max_attempts = "&cToo many failed login attempts."
login_timeout = "&cAuthentication timed out."

register_title = "&6Welcome, {username}!"
register_subtitle = "&7/register <password> <confirm>"
register_success = "&aAccount created successfully!"
register_password_mismatch = "&cPasswords do not match."
register_password_too_short = "&cPassword must be at least {min_length} characters."
register_password_too_long = "&cPassword must be at most {max_length} characters."
register_password_is_username = "&cPassword cannot be the same as your username."
register_password_blocked = "&cThat password is too common. Please choose a different one."
register_account_exists = "&cAn account already exists for this username."

reminder_title = "&6Please authenticate"
reminder_subtitle = "&7Use /login or /register"
unknown_command = "&7Available commands: &f/login&7, &f/register"
```

See the default config for the full list, which also covers `/changepassword`, `/unregister`, and admin command messages.

## Commands

### Player commands

| Command | Aliases | Description |
|---------|---------|-------------|
| `/login <password>` | `/l` | Authenticate with an existing account |
| `/register <password> <confirm>` | `/reg` | Create a new account |
| `/changepassword <old> <new>` | `/changepw`, `/cp` | Change your password |
| `/unregister <password>` | | Delete your account |
| `/cracked` | | Switch to cracked mode (use `/login` instead of auto-login) |
| `/premium` | | Switch back to premium auto-login |

`/cracked` and `/premium` are only registered when `[premium] enabled = true` and `allow_cracked_command = true`. Both require the player to reconnect for the change to take effect.

### Admin commands

These require either the `auth.admin` permission or a username in `admin_usernames`.

| Command | Description |
|---------|-------------|
| `/forcelogin <username>` | Force-authenticate a player stuck in auth limbo |
| `/forceunregister <username>` | Delete another player's account |
| `/forcechangepassword <username> <password>` | Reset another player's password |
| `/authreload` | Reload the auth config from disk |

## Account storage format

Accounts are stored in `plugins/auth/accounts.json`. Each account has:

| Field | Type | Description |
|-------|------|-------------|
| `username` | string | Lowercased canonical username |
| `display_name` | string | Original-case username |
| `password_hash` | string or null | Argon2id hash, or `null` for premium-only accounts |
| `registered_at` | datetime | When the account was created |
| `last_login` | datetime or null | Last successful login |
| `last_ip` | string or null | Last known IP |
| `login_count` | u64 | Total successful logins |
| `premium_info` | object or null | Present if the player has been detected as premium |

The `premium_info` object:

| Field | Type | Description |
|-------|------|-------------|
| `mojang_uuid` | UUID | The player's official Mojang UUID |
| `force_cracked` | bool | `true` if the player ran `/cracked` |
| `first_premium_login` | datetime | When the player was first detected as premium |
| `last_premium_login` | datetime or null | Last premium auto-login |

## How premium detection works

The detection pipeline has three layers:

1. A `DashMap` cache keyed by lowercase username, with a configurable TTL (`cache_ttl_seconds`). Cache hits avoid any network call.

2. `GET https://api.mojang.com/users/profiles/minecraft/<username>`. A 200 means the username is premium. A 404 means it's not. A 429 or network error triggers the `rate_limit_action` policy.

3. The API lookup alone is not trusted. It only determines whether to *attempt* the Mojang encryption handshake. The actual proof of identity is the RSA key exchange + `hasJoined` session verification, handled by the proxy core.

### Name squatting

If a cracked player registers "Steve" with a password, and then the real premium "Steve" connects:

1. The Mojang API says "Steve" is premium.
2. The proxy forces the encryption handshake. The real Steve's client proves ownership.
3. The auth handler sees signed textures and accepts.
4. The existing account is updated with `premium_info`. The cracked player's password hash is preserved.

If the cracked "Steve" tries to connect later, the proxy forces encryption again. The cracked client fails the handshake, but the plugin remembers the failure (see below), so on the next attempt they can join as cracked.

If the premium "Steve" later changes their Mojang username, the cracked player can connect again and `/login` with their password as before. Premium identity is tracked by Mojang UUID, not by username.

### Cracked players using premium usernames (second attempt)

A cracked player using a premium username (e.g. "Hypixel") will be kicked on their **first** connection. This is unavoidable: the proxy sends an encryption request, and the cracked client can't respond.

On the **second** connection, the plugin remembers the failure and sets `ForceOffline`. The player enters auth limbo and can `/register` or `/login` normally.

This works in both proxy modes:

| Mode | First connection | Second connection |
|------|-----------------|-------------------|
| `offline` + premium | ForceOnline → encryption fails → kick | ForceOffline (ignored, already offline) → auth limbo |
| `client_only` | Default Mojang auth → encryption fails → kick | ForceOffline → skip Mojang auth → auth limbo |

The remember window is controlled by `failed_auth_remember_seconds` (default: 10 minutes). After it expires, the proxy retries authentication once. If the player still can't complete it, they're remembered again. If they've since bought Minecraft, the auth succeeds and they get premium auto-login.

::: tip
This is the same approach used by FastLogin (`secondAttemptCracked`), LibreLogin, and other established Minecraft auth plugins. The first-connection kick is a protocol-level constraint that cannot be avoided.
:::

## Mojang session authentication

Mojang session auth is not part of this plugin. It's built into Infrarust's core and activates when you use `client_only` proxy mode.

In `client_only` mode, the proxy terminates the Minecraft connection and re-establishes it to the backend. During this process, Infrarust runs the standard Mojang authentication flow:

1. The proxy generates a 1024-bit RSA key pair at startup.
2. When a player connects, the proxy sends an `EncryptionRequest` with the public key and a random 4-byte verify token.
3. The client encrypts a shared secret and the verify token with the public key, then sends them back in an `EncryptionResponse`.
4. The proxy decrypts both values using its private key and checks the verify token matches.
5. The proxy computes a server hash (Minecraft's non-standard signed SHA-1 of the server ID, shared secret, and public key DER).
6. The proxy calls `sessionserver.mojang.com/session/minecraft/hasJoined?username=<name>&serverId=<hash>` to verify the player owns the account.
7. If Mojang confirms the session, the proxy enables AES/CFB8 encryption on the connection and returns the player's game profile (UUID, username, skin data).

Players using cracked or offline clients fail at step 6. With the auth plugin's premium detection enabled, cracked players get kicked on first connection but can join on their second attempt: the plugin remembers the failure and sets `ForceOffline` to bypass encryption.

The premium auto-login feature reuses this same flow in both directions. In `offline` mode, the proxy can switch a connection to online mode via `ForceOnline`. In `client_only` mode, it can switch to offline via `ForceOffline`. Both are controlled by plugins through the `PreLoginEvent`.
