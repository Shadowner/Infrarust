---
title: Auth Plugin
description: Password-based authentication for offline-mode Minecraft proxies using /login and /register commands
---

# Auth Plugin

The auth plugin adds password-based authentication to offline-mode proxies. When a player connects, they're held in a limbo state and must `/register` (first visit) or `/login` (returning player) before reaching the backend server.

This is separate from Mojang's session authentication, which Infrarust handles automatically in `client_only` proxy mode. The auth plugin exists for servers running in offline mode where Mojang's servers aren't verifying player identities.

## How it works

1. A player connects and enters the limbo state.
2. The plugin checks if an account exists for that username (case-insensitive).
3. New players see `/register <password> <confirm>`. Returning players see `/login <password>`.
4. A title and chat message remind the player what to do. Reminders repeat on an interval.
5. On success, the player leaves limbo and reaches the backend server.
6. On failure, the player gets feedback with remaining attempts. After too many failures, they're kicked.
7. If the player doesn't authenticate within the timeout, they're disconnected.

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

Passwords are hashed with Argon2id. If `migrate_legacy_hashes` is `true`, bcrypt hashes from older setups are automatically re-hashed to Argon2id on the next successful login.

The plugin also generates a dummy hash at startup. When a player tries to log in with a username that doesn't exist, the plugin still runs a full Argon2 verify against the dummy hash. This prevents attackers from measuring response times to determine which usernames have accounts.

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

### Admin commands

These require either the `auth.admin` permission or a username in `admin_usernames`.

| Command | Description |
|---------|-------------|
| `/forcelogin <username>` | Force-authenticate a player stuck in auth limbo |
| `/forceunregister <username>` | Delete another player's account |
| `/forcechangepassword <username> <password>` | Reset another player's password |
| `/authreload` | Reload the auth config from disk |

## Mojang session authentication

Mojang session auth is not part of this plugin. It's built into Infrarust's core and activates automatically when you use `client_only` proxy mode.

In `client_only` mode, the proxy terminates the Minecraft connection and re-establishes it to the backend. During this process, Infrarust runs the standard Mojang authentication flow:

1. The proxy generates a 1024-bit RSA key pair at startup.
2. When a player connects, the proxy sends an `EncryptionRequest` with the public key and a random 4-byte verify token.
3. The client encrypts a shared secret and the verify token with the public key, then sends them back in an `EncryptionResponse`.
4. The proxy decrypts both values using its private key and checks the verify token matches.
5. The proxy computes a server hash (Minecraft's non-standard signed SHA-1 of the server ID, shared secret, and public key DER).
6. The proxy calls `sessionserver.mojang.com/session/minecraft/hasJoined?username=<name>&serverId=<hash>` to verify the player owns the account.
7. If Mojang confirms the session, the proxy enables AES/CFB8 encryption on the connection and returns the player's game profile (UUID, username, skin data).

Players using cracked or offline clients will fail at step 6. If you need to accept those players, use `passthrough` or `offline` proxy mode with the auth plugin above.
