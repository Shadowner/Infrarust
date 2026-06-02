---
title: Web Interface
description: Admin dashboard and REST API for managing your Infrarust proxy from a browser
outline: [2, 3]
---

# Web Interface

Infrarust ships with a built-in web dashboard and REST API that lets you manage your proxy, players, servers, and bans from a browser or any HTTP client.

The web interface is powered by the `admin_api` plugin. It includes:

- A **Vue/Nuxt dashboard** with real-time updates
- A **REST API** with bearer-token authentication
- **Server-Sent Events** for live player activity and log streaming

## Enabling the Web Interface

Add a `[web]` section to your `infrarust.toml`:

```toml
[web]
enable_api = true    # REST API endpoints
enable_webui = true  # Web dashboard (SPA)
listen_port = 8080   # HTTP listen port
```

All three fields have defaults, so a bare `[web]` section is enough to enable both the API and the dashboard on port 8080.

::: tip
The `[web]` section must be present in `infrarust.toml` for the admin plugin to load. If the section is absent, the web interface is completely disabled.
:::

### Configuration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enable_api` | `bool` | `true` | Enables the REST API endpoints |
| `enable_webui` | `bool` | `true` | Serves the web dashboard SPA |
| `listen_port` | `u16` | `8080` | Port the HTTP server binds to |

## First Launch

On first start with `[web]` enabled, Infrarust generates a plugin config file at `plugins/admin_api/config.toml` containing a random API key:

```toml
bind = "127.0.0.1:8080"

# IMPORTANT: Change this API key before exposing the API
api_key = "a1b2c3d4-e5f6-7890-abcd-ef1234567890"

# CORS origins for the web dashboard (empty = no CORS)
# cors_origins = ["http://localhost:3000"]

# Rate limiting (requests per minute for authenticated endpoints)
# [rate_limit]
# requests_per_minute = 60
```

The generated API key is printed to the console log on first run. You need this key to log in to the dashboard.

::: warning
The API key must be at least 16 characters. Keep it secret. Anyone with the key has full control over your proxy.
:::

### Plugin Configuration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `bind` | `String` | `"127.0.0.1:8080"` | Address and port to bind the HTTP server |
| `api_key` | `String` | Auto-generated UUID | Bearer token for authentication |
| `cors_origins` | `Vec<String>` | `[]` | Allowed CORS origins (empty = no CORS) |
| `rate_limit.requests_per_minute` | `u64` | `60` | Max requests per minute per authenticated client |

## Dashboard

Open `http://localhost:8080` in your browser after starting Infrarust with `[web]` enabled. Log in with your API key.

The dashboard provides:

- **Overview** — player count, server count, uptime, and an activity feed
- **Players** — list connected players, kick, send to a server, or broadcast messages
- **Servers** — view all configured servers, check health, start/stop managed servers, and create servers via the API
- **Bans** — create, view, and remove bans (IP, username, or UUID)
- **Plugins** — list loaded plugins and their status
- **Logs** — real-time log console with level and target filtering

## REST API

All API endpoints are prefixed with `/api/v1`. Protected endpoints require a `Authorization: Bearer <api_key>` header.

### Public Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/health` | Health check (returns `{"status": "ok"}`) |

### Player Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/players` | List all connected players (paginated) |
| `GET` | `/api/v1/players/count` | Player count stats (total, by server, by mode) |
| `GET` | `/api/v1/players/{id_or_username}` | Get a single player's details |
| `POST` | `/api/v1/players/broadcast` | Broadcast a message to all players |
| `POST` | `/api/v1/players/{username}/kick` | Kick a player |
| `POST` | `/api/v1/players/{username}/send` | Send a player to another server |
| `POST` | `/api/v1/players/{username}/message` | Send a private message to a player |

### Server Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/servers` | List all servers |
| `GET` | `/api/v1/servers/{id}` | Get server details |
| `POST` | `/api/v1/servers` | Create a server (API-managed) |
| `PUT` | `/api/v1/servers/{id}` | Update a server |
| `DELETE` | `/api/v1/servers/{id}` | Delete a server |
| `POST` | `/api/v1/servers/{id}/start` | Start a managed server |
| `POST` | `/api/v1/servers/{id}/stop` | Stop a managed server |
| `GET` | `/api/v1/servers/{id}/health` | Ping the backend and check health |
| `GET` | `/api/v1/servers/{id}/health/cached` | Get cached health status |

### Ban Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/bans` | List all bans (paginated) |
| `GET` | `/api/v1/bans/check/{type}/{value}` | Check if a target is banned |
| `POST` | `/api/v1/bans` | Create a ban |
| `DELETE` | `/api/v1/bans/{type}/{value}` | Remove a ban |

### Plugin Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/plugins` | List all plugins |
| `GET` | `/api/v1/plugins/{id}` | Get plugin details |
| `POST` | `/api/v1/plugins/{id}/enable` | Enable a plugin |
| `POST` | `/api/v1/plugins/{id}/disable` | Disable a plugin |

### Proxy & Config Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/proxy` | Proxy status (version, uptime, player count) |
| `POST` | `/api/v1/proxy/shutdown` | Graceful proxy shutdown |
| `POST` | `/api/v1/proxy/gc` | Trigger garbage collection |
| `GET` | `/api/v1/stats` | Statistics overview |
| `GET` | `/api/v1/events/recent` | Recent activity events (last 100) |
| `GET` | `/api/v1/config/providers` | List configuration providers |
| `POST` | `/api/v1/config/reload` | Reload configuration |
| `GET` | `/api/v1/logs/history` | Log history (paginated) |

### Authentication Example

```bash
# Health check (no auth required)
curl http://localhost:8080/api/v1/health

# Get proxy status (auth required)
curl -H "Authorization: Bearer YOUR_API_KEY" \
     http://localhost:8080/api/v1/proxy

# Kick a player
curl -X POST \
     -H "Authorization: Bearer YOUR_API_KEY" \
     -H "Content-Type: application/json" \
     -d '{"reason": "AFK too long"}' \
     http://localhost:8080/api/v1/players/Steve/kick

# Create a ban
curl -X POST \
     -H "Authorization: Bearer YOUR_API_KEY" \
     -H "Content-Type: application/json" \
     -d '{"target": {"type": "username", "value": "griefer"}, "reason": "griefing"}' \
     http://localhost:8080/api/v1/bans
```

## Server-Sent Events (SSE)

Two SSE endpoints stream real-time data. Since `EventSource` cannot send HTTP headers, these endpoints authenticate via the `token` query parameter.

### Event Stream

```
GET /api/v1/events?token=YOUR_API_KEY&types=player.join,player.leave
```

Filter by event type using a comma-separated `types` parameter. If omitted, all event types are sent.

| Event Type | Description |
|------------|-------------|
| `player.join` | Player connected to the proxy |
| `player.leave` | Player disconnected |
| `player.switch` | Player moved to a different server |
| `server.state_change` | Server state changed (e.g. starting, online, stopped) |
| `config.reload` | Configuration was reloaded |
| `ban.created` | A new ban was created |
| `ban.removed` | A ban was removed |
| `stats.tick` | Periodic stats snapshot (every 5 seconds) |

### Log Stream

```
GET /api/v1/logs?token=YOUR_API_KEY&level=warn&target=infrarust_core
```

| Parameter | Default | Description |
|-----------|---------|-------------|
| `level` | `info` | Minimum log level (`trace`, `debug`, `info`, `warn`, `error`) |
| `target` | — | Filter by target prefix (e.g. `infrarust_core::proxy`) |

### Log History

```
GET /api/v1/logs/history?n=50&level=warn
```

| Parameter | Default | Description |
|-----------|---------|-------------|
| `n` | `100` | Number of entries to return (max 1000) |
| `level` | — | Minimum log level |
| `target` | — | Target prefix filter |

## Rate Limiting

Protected API endpoints are rate-limited. The default is 60 requests per minute. Rate limit status is returned in response headers:

| Header | Description |
|--------|-------------|
| `X-RateLimit-Limit` | Maximum requests per window |
| `X-RateLimit-Remaining` | Requests remaining in current window |
| `X-RateLimit-Reset` | Seconds until the window resets |
| `Retry-After` | Seconds to wait (only present when rate-limited) |

To change the limit, edit `plugins/admin_api/config.toml`:

```toml
[rate_limit]
requests_per_minute = 120
```

## Security

- The HTTP server binds to `127.0.0.1` by default. To expose it externally, change the `bind` address in `plugins/admin_api/config.toml` and place it behind a reverse proxy with TLS.
- API key verification uses constant-time comparison to prevent timing attacks.
- All authentication failures are logged to the `audit` target.
- CORS is disabled by default. Add origins to `cors_origins` if the dashboard is served from a different domain.

::: danger
Do not expose the admin API to the public internet without TLS and a strong API key. Use a reverse proxy (nginx, Caddy, etc.) to terminate HTTPS.
:::

## Folder Structure

After enabling the web interface, the plugin creates the following files:

```
infrarust/
├── infrarust.toml              # [web] section enables the plugin
└── plugins/
    └── admin_api/
        ├── config.toml         # API key, bind address, rate limits
        └── servers.json        # API-created servers (persistent)
```
