---
title: Admin API & Web Interface
description: Built-in REST API and web dashboard for managing your Infrarust proxy. Monitor players, servers, bans, and stream logs in real time.
---

# Admin API & Web Interface

The admin API plugin exposes a REST API and an embedded web dashboard for managing your Infrarust proxy over HTTP. You can list players, kick or move them between servers, manage bans, edit server and proxy configuration, drain a backend address for maintenance, and stream live events and logs, all without touching the Minecraft client or the terminal.

The plugin is always compiled into the binary. It activates when `infrarust.toml` has a `[web]` section and `enable_api` is left on.

Anything the API creates is stored in its own data directory, `<plugins_dir>/admin_api/`. It reads configuration from every provider but only writes servers it owns, plus `infrarust.toml` through the global config endpoints. Your `servers_dir` stays under the file provider's control.

## Enabling the plugin

Add a `[web]` section to your `infrarust.toml`:

```toml
[web]
```

That is enough. All fields have defaults:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enable_api` | bool | `true` | Start the REST API. `false` leaves the plugin unregistered and nothing binds. |
| `enable_webui` | bool | follows `enable_api` | Serve the embedded web dashboard. Cannot be `true` while `enable_api` is `false`. |
| `bind` | string | `"127.0.0.1:8080"` | Socket address the HTTP server listens on |
| `api_key` | string | *(see below)* | Bearer token for authentication. Must be at least 16 characters. |
| `cors_origins` | string[] | `[]` | Allowed CORS origins. Empty means no CORS headers are sent. |
| `rate_limit.requests_per_minute` | u64 | `60` | Maximum requests per minute across all clients on authenticated endpoints |

The dashboard is served by the same HTTP server as the API and calls it for every screen, so `enable_webui = true` with `enable_api = false` is refused at startup. To turn the whole thing off, set both to `false` or drop the `[web]` section.

To change the bind address or disable the web UI while keeping the API:

```toml
[web]
bind = "127.0.0.1:9090"
enable_webui = false
```

### API key behavior

If `api_key` is not set and `bind` resolves to a loopback address (`127.0.0.1`, `::1`, `localhost`), the plugin generates a random UUID v4 key at startup and logs it as a warning:

```
WARN No API key configured for loopback bind (127.0.0.1:8080) — generated an ephemeral key: a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

This key is not written to disk. It changes on every restart. For a persistent key, set one explicitly:

```toml
[web]
api_key = "your-strong-key-here"
```

::: warning
If `bind` is set to a non-loopback address (e.g. `0.0.0.0:8080`) and no `api_key` is configured, the plugin refuses to start. You must supply a key of at least 16 characters before binding to any externally reachable address.
:::

::: danger
Do not expose the admin API to the public internet without a reverse proxy or firewall. The default bind `127.0.0.1` restricts access to the local machine.
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

Authenticated endpoints are rate-limited to `requests_per_minute` (default 60). The counter is shared across all clients. It tracks total requests to the API, not per-IP. The health endpoint is exempt.

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
| GET | `/api/v1/servers` | List every configured server, whatever provider it came from |
| GET | `/api/v1/servers/{id}` | Routing summary, state, and connected players |
| GET | `/api/v1/servers/{id}/config` | The whole server config as JSON |
| GET | `/api/v1/servers/{id}/raw` | The server's TOML document, as `text/plain` |
| POST | `/api/v1/servers` | Create a server from a full config as JSON |
| PUT | `/api/v1/servers/{id}` | Replace a server config from JSON |
| PUT | `/api/v1/servers/{id}/raw` | Replace a server config from a TOML body |
| POST | `/api/v1/servers/validate` | Check a server config without saving it |
| DELETE | `/api/v1/servers/{id}` | Delete a server the API owns |
| POST | `/api/v1/servers/{id}/start` | Start a server |
| POST | `/api/v1/servers/{id}/stop` | Stop a server |
| GET | `/api/v1/servers/{id}/health` | Live health check (pings the Minecraft server, 5s timeout) |
| GET | `/api/v1/servers/{id}/health/cached` | Last cached health check result |

`GET /api/v1/servers` and `GET /api/v1/servers/{id}` return a `source` field carrying the provider type the config came from (`file`, `docker`, `plugin:admin_api:api`) and an `editable` boolean.

`POST` and `PUT` take a full `ServerConfig` document, the same shape as a file under `servers_dir`. `PUT` is a full replace: any field you leave out goes back to its default. The body must identify itself as the server you are writing to, either through `id`/`name` or by omitting both, and `POST` refuses a body with neither since there would be nothing to name the file after.

`POST /api/v1/servers/validate` and `POST /api/v1/config/proxy/validate` read the body as TOML when the request carries a `toml` or `text/*` content type, and as JSON otherwise. Both always answer `200` with `{"valid": bool, "errors": [], "warnings": []}`. Server validation fills `warnings` with the same balancing hints Infrarust logs at startup, such as several addresses left on `first_available`, or an address with `weight = 0`. A proxy config has no warning source, so its `warnings` array is always empty.

### API-managed servers

Servers created through the API live in `<plugins_dir>/admin_api/servers/`, one `.toml` file per server holding a full server config document. The proxy's own `servers_dir` is never written to, and neither is `infrarust.toml` unless you call the global config endpoints below. Turn the API off with `enable_api = false` and the rest of your configuration is exactly as you left it.

The file name without its extension is what the plugin files a server under, and what its change events are keyed by. The routed id comes from the document's `name`, then its `id`, then the file name. Anything the API creates is named after the id it resolved, so the two agree; a file you write by hand can disagree, and the routed id wins.

Read endpoints work for every server. `GET /api/v1/servers/{id}/raw` serves a file-provided or Docker-provided config just as happily as one the API owns, so the dashboard can show a config it cannot save. Writes are refused for anything the plugin does not own:

| Response | When |
|----------|------|
| `403 Forbidden` | The server exists but comes from another provider. The message names it. |
| `404 Not Found` | No server with that id. |
| `409 Conflict` | `POST` on an id that already exists anywhere. |
| `400 Bad Request` | Malformed TOML or JSON, a failed validation, or a body whose id does not match the path. |

`PUT /api/v1/servers/{id}/raw` writes the bytes you send to disk untouched, comments and layout included, while the JSON `PUT` regenerates the document from the parsed config and drops them. Reads do not round-trip either way: `GET /api/v1/servers/{id}/raw` renders the config the proxy is routing on rather than the file behind it, so what comes back is normalized TOML with no comments in it. Open the file to see it as you wrote it.

### Hot reload

The plugin watches its servers directory and applies changes without a restart. A file you drop in, edit, or delete by hand reaches the proxy the same way an API write does, after a 200 ms debounce that collapses an editor's write burst into one reload. A file that does not parse or does not validate is logged and skipped, and the previous version of that server stays live.

`POST /api/v1/config/reload` forces the same rescan and answers with what changed:

```json
{
  "data": {
    "success": true,
    "message": "Reloaded API-managed servers: 1 added, 0 updated, 0 removed",
    "details": { "added": 1, "updated": 0, "removed": 0 }
  }
}
```

It only covers this directory. Reloading `servers_dir`, `infrarust.toml`, or Docker labels is not part of it.

A pre-2.0 `servers.json` in the data directory is converted to individual `.toml` files on the first start and renamed to `servers.json.migrated`.

### Backends

Per-address load balancing status and drain controls for servers that list several `addresses`. See [Load balancing](../../configuration/load-balancing) for the balancing itself.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/servers/{id}/backends` | Strategy and per-address status for one server |
| GET | `/api/v1/health/backends` | The same, for every server, keyed by server id |
| POST | `/api/v1/servers/{id}/backends/{address}/drain` | Take an address out of rotation |
| POST | `/api/v1/servers/{id}/backends/{address}/enable` | Put it back in |
| POST | `/api/v1/servers/{id}/backends/{address}/reset` | Forget its failure history |

```json
{
  "data": {
    "strategy": "least_conn",
    "backends": [
      {
        "address": "10.0.0.1:25565",
        "weight": 2,
        "effective_weight": 1,
        "state": "healthy",
        "active_connections": 4,
        "healthy_since_secs": 30,
        "ejections": 1,
        "last_failure_secs_ago": 90
      }
    ]
  }
}
```

`strategy` is `first_available`, `round_robin`, or `least_conn`. `effective_weight` is the weight selection actually uses, so it sits below `weight` while an address ramps through slow start. `healthy_since_secs` counts from the moment the address became healthy, which is what the ramp measures against; it is absent for an ejected address, and for one that has been stable long enough for the proxy to stop tracking it.

| `state` | Meaning |
|---------|---------|
| `healthy` | In rotation |
| `probing` | Ejected, but its backoff has elapsed, so one recovery attempt is allowed |
| `unhealthy` | Ejected and still inside its backoff |
| `draining` | Taken out of rotation by an operator |

The `{address}` segment is one path segment in `host:port` form, percent-encoded. IPv6 hosts are bracketed (`[::1]:25565`), which is the form the read endpoints emit, so a response address pastes straight into a mutation URL. An address that does not parse gives `400`; an address that is not in that server's config gives `404`.

Draining stops new sessions from reaching an address. Players already on it stay connected. When every other address of the server is ejected, Infrarust reinstates the ejected ones rather than the drained one, so a drain you set for maintenance is respected as long as anything else can carry traffic. Drain every address of a server and the drain is ignored for all of them, which keeps a drain from black-holing the server.

Drain intent is stored in `<plugins_dir>/admin_api/drained.json` and replayed after a restart. Health state itself is in-memory and rebuilt from scratch each start, so without that file a maintenance drain would silently disappear on the next restart.

`reset` clears the failure counters, the ejection count, and the backoff. An ejected address rejoins the rotation and ramps back up through slow start; one that was already healthy only loses its history. It leaves drain state alone, so an address you drained stays drained after a reset.

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
| GET | `/api/v1/config/providers` | Active config providers and how many servers each one supplies |
| POST | `/api/v1/config/reload` | Rescan the API-managed server directory |
| GET | `/api/v1/config/proxy` | The config the proxy is running on as JSON, CLI overrides applied and defaults filled in |
| GET | `/api/v1/config/proxy/raw` | The global `infrarust.toml` as TOML |
| PUT | `/api/v1/config/proxy/raw` | Rewrite `infrarust.toml` from a TOML body |
| POST | `/api/v1/config/proxy/validate` | Check a global config without saving it |

`GET /api/v1/config/providers` counts the servers currently routed, grouped by provider type: `[{"provider_type": "file", "configs_count": 4}, {"provider_type": "plugin:admin_api:api", "configs_count": 1}]`.

Reads of the global config replace `web.api_key` with `<redacted>`, and a write restores the stored key, so fetching the config and saving it back cannot destroy the key. Sending a real `api_key` sets it. Deleting the whole `[web]` table deletes it, key included. A write that carries `<redacted>` with no stored key to restore is rejected.

Server documents get the same treatment: `GET /api/v1/servers/{id}/raw` and `GET /api/v1/servers/{id}/config` replace `server_manager.api_key` with `<redacted>`, and a `PUT` back to either endpoint restores the stored credential. Creating a server with `<redacted>` as its key is rejected, since there is nothing to restore from.

`GET /api/v1/config/proxy` reports what the process is actually running on, so a `--bind` or `--servers-dir` override shows up there but not in the file. `GET /api/v1/config/proxy/raw` returns the file as it is on disk, comments and all, and `PUT` keeps the document you send rather than merging it into the old one, so deleting a key works. The write is validated before it lands and goes through a temporary file, so a crash mid-write cannot truncate `infrarust.toml`. If the file is missing or unparsable, both read and write fall back to the configuration the proxy started with.

Writing the global config never touches the running proxy. The response carries `requires_restart: true` and the new configuration applies on the next start:

```json
{
  "data": {
    "success": true,
    "message": "Proxy config written, restart the proxy to apply it",
    "requires_restart": true
  }
}
```

::: warning Global config writes need a capability
`PUT /api/v1/config/proxy/raw` requires the plugin to hold `Capability::ConfigWrite`. The admin API is a native plugin compiled into the binary, so it is trusted and holds every capability by default. Without it the plugin receives a read-only config service and the endpoint answers `403`. See [Plugin capabilities](../wasm/capabilities).
:::

Nothing outside `<plugins_dir>/admin_api/` and `infrarust.toml` is ever written. Files under `servers_dir` belong to the file provider and the API only reads them.

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
| `backend.health_change` | A backend address is ejected, recovers, or changes drain state |
| `stats.tick` | Periodic stats snapshot (every 5 seconds) |

Omit the `types` parameter to receive all events. The stream sends a keep-alive comment every 15 seconds.

`backend.health_change` carries the address, every server that lists it, and the new state:

```json
{
  "address": "10.0.0.1:25565",
  "server_ids": ["lobby", "survival"],
  "state": "draining",
  "timestamp": "2025-01-15T10:30:00Z"
}
```

The state is the one selection acts on, so a drained address reports `draining` even while its own health checks pass.

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
  -H "Authorization: Bearer YOUR_API_KEY" \
  http://127.0.0.1:8080/api/v1/players | jq
```

```json
{
  "data": [
    {
      "id": 1,
      "username": "Steve",
      "uuid": "069a79f4-44e9-4726-a5be-fca90e38aaf5",
      "ip": "203.0.113.7",
      "server": "survival",
      "is_active": true,
      "connected_since": "2025-01-15T10:30:00Z",
      "connected_duration": "1h 12m 5s"
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
