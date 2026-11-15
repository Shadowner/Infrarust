---
title: Rate Limiting
description: Configure per-IP rate limits for login attempts and status pings in Infrarust.
---

# Rate Limiting

Infrarust rate-limits incoming connections per client address: one IPv4 address, or one IPv6 /64. Every connection passes through the `rate_limiter` middleware before it reaches domain routing or any backend server.

Two independent buckets exist for each IP: one for **login/transfer** attempts and one for **status pings**. Status pings happen when the Minecraft client refreshes the server list, so they fire much more often than actual logins. Splitting the buckets prevents server-list refreshes from locking players out of joining.

## Configuration

Rate limiting is **enabled by default**. To change the limits, add a `[rate_limit]` section to your `infrarust.toml`:

```toml
[rate_limit]
enabled = true
max_connections = 10     # logins per IP per window
window = "10s"           # login window duration
status_max = 300         # status pings per IP per window
status_window = "10s"    # status ping window duration
```

All fields are optional. If you omit the entire `[rate_limit]` section, Infrarust uses the defaults shown below. Set `enabled = false` to turn the rate limiter off.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `true` | Enable the rate limiter |
| `max_connections` | integer | `10` | Max login/transfer connections per IP in the window |
| `window` | duration | `10s` | Time window for the login bucket |
| `status_max` | integer | `300` | Max status pings per IP in the window |
| `status_window` | duration | `10s` | Time window for the status bucket |

Duration values use the `humantime` format: `10s`, `1m`, `500ms`, `2m30s`.

`max_connections` here counts login attempts over time. It is not the top-level `max_connections`, which caps open connections, nor `max_connections_per_ip`, which caps the connections one address holds at the same time. See [global configuration](../global).

### Clients that share one address

::: warning
The rate limiter counts by the address Infrarust sees. When players reach the proxy through a tunnel or a NAT, they all arrive from the same address and share one login bucket: 10 logins per 10 seconds for everyone together. This is the case behind playit, frp, ngrok, the Docker userland proxy, carrier-grade NAT, and Kubernetes services that rewrite the source address.

The same players also share the `max_connections_per_ip` limit (32 open connections by default).

Pick one fix:

- Have the tunnel or load balancer send the real address with PROXY protocol, and set `receive_proxy_protocol = true` with `trusted_proxies`. See [Proxy Protocol](./proxy-protocol).
- Raise `max_connections` and `status_max` in `[rate_limit]`, and `max_connections_per_ip`, to fit the number of players behind that address.
- Set `enabled = false` in `[rate_limit]` and `max_connections_per_ip = 0` when something in front of Infrarust already limits connections.
:::

The first time the rate limiter refuses a connection, it logs this warning once:

```
refusing connections over [rate_limit]; clients behind playit, frp, ngrok, Docker userland proxy or NAT share one address: send the real address with receive_proxy_protocol and trusted_proxies, raise max_connections and status_max, or set enabled = false. This warning is logged once
```

## How it works

The rate limiter uses a token-bucket algorithm (via the `governor` crate). Each IP gets a bucket that refills at a steady rate over the window. A burst up to the configured maximum is allowed, then additional connections are rejected until tokens replenish.

When a login exceeds the limit, the client receives a "Rate limit exceeded" disconnect message, and a status ping is closed without a reply. Each refusal is logged at `debug` level with the source IP and connection intent; only the first one is also logged at `warn`.

### Login vs status

The Minecraft protocol declares an intent in the handshake packet: `Status`, `Login`, or `Transfer`. Infrarust reads this intent and routes the connection to the matching bucket:

- `Login` and `Transfer` use `max_connections` / `window`
- `Status` uses `status_max` / `status_window`

This means a player who refreshes their server list repeatedly won't consume any login tokens.

## Examples

### Strict login limits, relaxed status

Allow only 2 logins per 30 seconds, but let status pings flow freely:

```toml
[rate_limit]
enabled = true
max_connections = 2
window = "30s"
status_max = 100
status_window = "10s"
```

### Tighten both buckets

Useful if you're under a bot attack that targets both status and login:

```toml
[rate_limit]
enabled = true
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
2. Handshake parser
3. Ban IP check
4. **Rate limiter**
5. Domain router

Connections refused by the global `[ip_filter]` and status pings from banned addresses are rejected before they consume any rate-limit tokens. A server's own `ip_filter` is checked by the domain router, after the rate limiter. Legacy clients (Beta to 1.6) go through the same pipeline and are rate-limited like modern ones.
