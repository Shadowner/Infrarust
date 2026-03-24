---
title: Rate Limiting
description: Configure per-IP rate limits for login attempts and status pings in Infrarust.
---

# Rate Limiting

Infrarust rate-limits incoming connections per IP address. Every connection passes through the `rate_limiter` middleware before it reaches domain routing or any backend server.

Two independent buckets exist for each IP: one for **login/transfer** attempts and one for **status pings**. Status pings happen when the Minecraft client refreshes the server list, so they fire much more often than actual logins. Splitting the buckets prevents server-list refreshes from locking players out of joining.

## Configuration

Add a `rate_limit` section to your `infrarust.toml`:

```yaml
[rate_limit]
max_connections = 3      # logins per IP per window
window = "10s"           # login window duration
status_max = 30          # status pings per IP per window
status_window = "10s"    # status ping window duration
```

All four fields are optional. If you omit the entire `[rate_limit]` section, Infrarust uses the defaults shown above.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `max_connections` | integer | `3` | Max login/transfer connections per IP in the window |
| `window` | duration | `10s` | Time window for the login bucket |
| `status_max` | integer | `30` | Max status pings per IP in the window |
| `status_window` | duration | `10s` | Time window for the status bucket |

Duration values use the `humantime` format: `10s`, `1m`, `500ms`, `2m30s`.

## How it works

The rate limiter uses a token-bucket algorithm (via the `governor` crate). Each IP gets a bucket that refills at a steady rate over the window. A burst up to the configured maximum is allowed, then additional connections are rejected until tokens replenish.

When a connection exceeds the limit, the client receives a "Rate limit exceeded" disconnect message and the event is logged at `debug` level with the source IP and connection intent.

### Login vs status

The Minecraft protocol declares an intent in the handshake packet: `Status`, `Login`, or `Transfer`. Infrarust reads this intent and routes the connection to the matching bucket:

- `Login` and `Transfer` use `max_connections` / `window`
- `Status` uses `status_max` / `status_window`

This means a player who refreshes their server list 30 times won't consume any login tokens.

## Examples

### Strict login limits, relaxed status

Allow only 2 logins per 30 seconds, but let status pings flow freely:

```yaml
[rate_limit]
max_connections = 2
window = "30s"
status_max = 100
status_window = "10s"
```

### Tighten both buckets

Useful if you're under a bot attack that targets both status and login:

```yaml
[rate_limit]
max_connections = 1
window = "5s"
status_max = 5
status_window = "5s"
```

::: warning
Setting `max_connections` to `1` with a short window can lock out players on shared networks (NAT, university, VPN) where multiple players share the same public IP.
:::

## Pipeline position

The rate limiter runs early in the connection pipeline, right after the handshake is parsed and before domain routing. The common pipeline order is:

1. IP filter
2. Ban IP check
3. Handshake parser
4. **Rate limiter**
5. Domain router

IP-filtered and banned connections are rejected before they consume any rate-limit tokens.
