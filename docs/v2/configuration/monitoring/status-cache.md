---
title: Status Cache
description: Configure MOTD caching to reduce backend load and protect against status ping floods.
---

# Status Cache

When a player opens the Minecraft server list, their client sends a status ping to fetch the MOTD, player count, favicon, and version. Without caching, every one of those pings hits your backend servers directly.

The status cache stores each server's last known response for a configurable TTL. While the entry is fresh, Infrarust answers status pings from cache without contacting the backend at all.

## Configuration

Add a `status_cache` section to your `infrarust.toml`:

```toml{2-3}
[status_cache]
ttl = "5s"
max_entries = 1000
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `ttl` | duration | `5s` | How long a cached response stays fresh. Uses `humantime` format (`5s`, `30s`, `2m`). |
| `max_entries` | integer | `1000` | Soft bound on cached server entries. The field is accepted and stored, but the internal cache does not currently enforce a hard eviction limit. |

The `ttl` controls how often Infrarust actually connects to your backend for status. A 5-second TTL means at most one backend connection per server every 5 seconds, regardless of how many clients request status in that window.

## How the cache works

The status handler follows this decision tree for each incoming status ping:

1. If the server has a fresh cache entry (within TTL), return it immediately.
2. If no fresh entry exists, wait for the server's relay slot. Only one relay runs per server at a time, and a request waits at most 1.5 seconds for its turn.
3. If the wait runs out, or the last relay to this server failed less than 3 seconds ago, answer with the fallback below without contacting the backend.
4. Otherwise check fresh again (another request may have just populated the cache), then relay the ping to the backend and store the response.
5. If the relay fails, remember the failure for 3 seconds and answer with the fallback.

The fallback is the first of these that exists:

1. The server's `[motd.unreachable]` entry.
2. A stale cache entry (expired but still stored).
3. The `unreachable` entry of the global `default_motd`.
4. A synthetic "Server unreachable" MOTD.

The relay slot prevents cache stampede: when 50 clients ping at the same moment and the cache has expired, only one relay runs. The other 49 wait for it. If it succeeds within 1.5 seconds they read its result from cache. If it fails, they get the fallback. If it is still running after 1.5 seconds, they get the fallback and the relay goes on for the next requests.

The slot also keeps pings from queueing behind a backend that accepts connections and never answers. One ping waits for the backend, the others give up after 1.5 seconds, and once the relay has failed no ping contacts that backend for the next 3 seconds.

Legacy pings (Beta to 1.6) share the same slot and the same failure memory as modern pings, so a server gets one relay at a time across both. A legacy ping that falls back is answered with a MOTD built from the configuration; it does not use the stale entry.

## DDoS and ping floods

Status pings are the cheapest request an attacker can make. A single machine can generate thousands of pings per second, and each one normally opens a TCP connection to your backend.

The status cache is your first line of defense. With a 5-second TTL, an attacker sending 10,000 pings/second only causes one backend connection every 5 seconds per server. The other 49,999 pings are answered from memory.

For stronger protection, combine the cache with the rate limiter. The `rate_limit` section has separate settings for status pings:

```toml
[rate_limit]
status_max = 300
status_window = "10s"
```

This limits each IP address to 300 status pings per 10-second window. Clients that exceed the limit are dropped before they reach the cache.

::: tip
The default TTL of 5 seconds works well for most setups. Players refreshing their server list won't notice a 5-second delay in player count updates. If you run a large public server and see heavy ping traffic in your logs, increase the TTL to `15s` or `30s`.
:::

## Stale fallback

When a backend goes down, Infrarust keeps serving the last known response instead of showing an error. This stale fallback has no separate TTL; entries remain in memory until the proxy restarts or a config reload happens (which clears the entire cache).

This means players still see a reasonable MOTD and player count in their server list even during brief backend outages or restarts.

## Interaction with MOTD overrides

If you define custom MOTDs in your server config, they are applied on top of the cached (or relayed) response. The `motd.online` entry overrides fields in the real backend response:

```toml
[motd.online]
text = "§aWelcome to my server"
max_players = 200
```

When the cache returns a response, Infrarust patches it with your overrides before sending it to the client. The cached data still provides the real player count and version.

For non-online states (sleeping, starting, crashed, stopping, unreachable), Infrarust skips the cache entirely and returns a synthetic response built from the corresponding MOTD entry. See [Server Configuration](../servers.md) for the full MOTD options.
