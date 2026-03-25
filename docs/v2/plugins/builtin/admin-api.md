---
title: Admin API & Web Interface
description: Built-in REST API and web dashboard for managing your Infrarust proxy — monitor players, servers, bans, and stream logs in real time.
---

# Admin API & Web Interface

The admin API plugin exposes a REST API and an embedded web dashboard for managing your Infrarust proxy over HTTP. You can list players, kick or move them between servers, manage bans, check server health, reload configuration, and stream live events and logs — all without touching the Minecraft client or the terminal.

The plugin is always compiled into the binary. It activates when a `[web]` section is present in `infrarust.toml`.

## Enabling the plugin

Add a `[web]` section to your `infrarust.toml`:

```toml
[web]
```

That's it. All three fields have defaults:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enable_api` | bool | `true` | Start the REST API |
| `enable_webui` | bool | `true` | Serve the embedded web dashboard |
| `listen_port` | u16 | `8080` | Port the HTTP server binds to |

To change the port or disable the web UI while keeping the API:

```toml
[web]
listen_port = 9090
enable_webui = false
```

On first start, the plugin generates `plugins/admin_api/config.toml` with a random API key. You'll see it in the logs:

```
INFO Generated admin API key: a1b2c3d4-e5f6-7890-abcd-ef1234567890
INFO Config written to plugins/admin_api/config.toml
```

::: warning
Save your API key somewhere safe. It is the only credential protecting the admin API.
:::

## Plugin configuration

The plugin reads its own config from `plugins/admin_api/config.toml`:

```toml
bind = "127.0.0.1:8080"

# API key for authentication (auto-generated, min 16 characters)
api_key = "a1b2c3d4-e5f6-7890-abcd-ef1234567890"

# CORS origins for the web dashboard (empty = no CORS headers)
# cors_origins = ["http://localhost:3000"]

# Rate limiting for authenticated endpoints
# [rate_limit]
# requests_per_minute = 60
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `bind` | string | `"127.0.0.1:8080"` | Address the HTTP server listens on |
| `api_key` | string | *(auto-generated)* | Bearer token for authentication. Must be at least 16 characters. |
| `cors_origins` | string[] | `[]` | Allowed CORS origins. Empty means no CORS headers are sent. |
| `rate_limit.requests_per_minute` | u64 | `60` | Maximum requests per minute per client on authenticated endpoints |

::: danger
Do not expose the admin API to the public internet without a reverse proxy or firewall. The default bind address `127.0.0.1` restricts access to the local machine.
:::

## Authentication

All endpoints except `GET /api/v1/health` require a Bearer token in the `Authorization` header:

```bash
curl -H "Authorization: Bearer YOUR_API_KEY" http://127.0.0.1:8080/api/v1/proxy
```

The token is compared using constant-time verification to prevent timing attacks.

### SSE authentication

Server-Sent Events endpoints (`/api/v1/events`, `/api/v1/logs`) cannot use the `Authorization` header because the browser `EventSource` API does not support custom headers. These endpoints authenticate via a `token` query parameter instead:

```
GET /api/v1/events?token=YOUR_API_KEY&types=player.join,player.leave
```

## Rate limiting

Authenticated endpoints are rate-limited to `requests_per_minute` (default 60). The health endpoint is exempt.

Response headers on every authenticated request:

| Header | Description |
|--------|-------------|
| `X-RateLimit-Limit` | Allowed requests per minute |
| `X-RateLimit-Remaining` | Remaining requests in the current window |
| `X-RateLimit-Reset` | Seconds until the window resets |

When the limit is exceeded, the API returns `429 Too Many Requests` with a `Retry-After` header.

## API reference

All responses follow a consistent format:

::: code-group

```json [Success]
{
  "data": { ... }
}
```

```json [Paginated]
{
  "data": [ ... ],
  "meta": {
    "total": 42,
    "page": 1,
    "per_page": 20,
    "total_pages": 3
  }
}
```

```json [Error]
{
  "error": {
    "code": "NOT_FOUND",
    "message": "Player 'Steve' not found"
  }
}
```

:::

Paginated endpoints accept `?page=1&per_page=20` query parameters. Maximum `per_page` is 100.

### Public endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/health` | Health check. Returns `{"status": "ok"}`. No auth required. |

### Proxy

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/proxy` | Proxy status: version, uptime, player count, server count, features, memory usage |
| POST | `/api/v1/proxy/shutdown` | Graceful proxy shutdown |
| POST | `/api/v1/proxy/gc` | Trigger garbage collection (no-op in Rust, returns success) |

### Players

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/players` | List online players (paginated, filterable by `server` and `mode`) |
| GET | `/api/v1/players/count` | Player count grouped by server and proxy mode |
| GET | `/api/v1/players/{id_or_username}` | Get a specific player's details |
| POST | `/api/v1/players/broadcast` | Broadcast a message to all online players |
| POST | `/api/v1/players/{username}/kick` | Kick a player |
| POST | `/api/v1/players/{username}/send` | Transfer a player to another server |
| POST | `/api/v1/players/{username}/message` | Send a chat message to a player |

### Servers

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/servers` | List all configured servers |
| GET | `/api/v1/servers/{id}` | Get a server's configuration and state |
| POST | `/api/v1/servers` | Create a server (API-managed, stored in `plugins/admin_api/servers.json`) |
| PUT | `/api/v1/servers/{id}` | Update a server's configuration |
| DELETE | `/api/v1/servers/{id}` | Delete an API-managed server |
| POST | `/api/v1/servers/{id}/start` | Start a server |
| POST | `/api/v1/servers/{id}/stop` | Stop a server |
| GET | `/api/v1/servers/{id}/health` | Real-time health check (pings the Minecraft server, 5s timeout) |
| GET | `/api/v1/servers/{id}/health/cached` | Last cached health check result |

### Bans

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/bans` | List all bans (paginated) |
| GET | `/api/v1/bans/check/{target_type}/{value}` | Check if a username, UUID, or IP is banned |
| POST | `/api/v1/bans` | Create a ban. Target types: `username`, `uuid`, `ip` |
| DELETE | `/api/v1/bans/{target_type}/{value}` | Remove a ban |

### Plugins

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/plugins` | List all loaded plugins |
| GET | `/api/v1/plugins/{id}` | Get a specific plugin's info |
| POST | `/api/v1/plugins/{id}/enable` | Enable a plugin |
| POST | `/api/v1/plugins/{id}/disable` | Disable a plugin |

### Configuration

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/config/providers` | List active configuration providers |
| POST | `/api/v1/config/reload` | Reload proxy configuration from disk |

### Statistics

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/stats` | Overview: players online, servers total, uptime, breakdowns by server and state |
| GET | `/api/v1/events/recent` | Last 100 activity events (excludes stats ticks) |

### Logs

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/logs/history` | Recent log entries from the ring buffer. Query: `?n=100&level=warn&target=infrarust_core` |

## Server-Sent Events (SSE)

Two streaming endpoints provide real-time data without polling. Both use query-parameter authentication.

### Event stream

```
GET /api/v1/events?token=YOUR_API_KEY&types=player.join,player.leave
```

Available event types:

| Event type | Fired when |
|------------|------------|
| `player.join` | A player connects |
| `player.leave` | A player disconnects |
| `player.switch` | A player moves between servers |
| `server.state_change` | A server's state changes |
| `config.reload` | Configuration is reloaded |
| `ban.created` | A ban is created |
| `ban.removed` | A ban is removed |
| `stats.tick` | Periodic stats snapshot (every 5 seconds) |

Omit the `types` parameter to receive all events. The stream sends a keep-alive comment every 15 seconds.

### Log stream

```
GET /api/v1/logs?token=YOUR_API_KEY&level=warn&target=infrarust_core
```

Streams log entries in real time. Filter by minimum `level` (`trace`, `debug`, `info`, `warn`, `error`) and `target` module prefix.

## Web dashboard

When `enable_webui` is `true`, the plugin serves an embedded web frontend at the root URL (`http://127.0.0.1:8080/`). The frontend is a Nuxt SPA bundled into the binary at compile time.

Non-API routes serve static files from the embedded bundle. If a requested file doesn't exist, the server returns `index.html` for client-side routing. API routes (`/api/*`) that don't match a defined endpoint return 404.

Cache headers:

| Path pattern | Cache-Control |
|-------------|---------------|
| `_nuxt/*` | `public, max-age=31536000, immutable` |
| `index.html`, `200.html` | `no-cache` |
| Other static files | `public, max-age=3600` |

## Example: list online players

```bash
curl -s \
  -H "Authorization: Bearer $(cat plugins/admin_api/config.toml | grep api_key | cut -d'"' -f2)" \
  http://127.0.0.1:8080/api/v1/players | jq
```

```json
{
  "data": [
    {
      "id": 1,
      "username": "Steve",
      "uuid": "069a79f4-44e9-4726-a5be-fca90e38aaf5",
      "server": "survival",
      "proxy_mode": "client_only",
      "connected_at": "2025-01-15T10:30:00Z"
    }
  ],
  "meta": {
    "total": 1,
    "page": 1,
    "per_page": 20,
    "total_pages": 1
  }
}
```

## Example: kick a player

```bash
curl -X POST \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"reason": "Server maintenance"}' \
  http://127.0.0.1:8080/api/v1/players/Steve/kick
```

## Example: subscribe to events

```javascript
const events = new EventSource(
  'http://127.0.0.1:8080/api/v1/events?token=YOUR_API_KEY&types=player.join,player.leave'
);

events.addEventListener('player.join', (e) => {
  const data = JSON.parse(e.data);
  console.log(`${data.username} joined ${data.server}`);
});

events.addEventListener('player.leave', (e) => {
  const data = JSON.parse(e.data);
  console.log(`${data.username} left`);
});
```
