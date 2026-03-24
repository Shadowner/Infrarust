---
title: Domain Routing
description: How Infrarust routes players to backend servers based on the domain they connect with.
---

# Domain Routing

Infrarust routes incoming connections to backend servers based on the domain name in the Minecraft handshake packet. When a player connects to `survival.mc.example.com`, the proxy looks up which server config owns that domain and forwards the connection there.

## How it works

Every Minecraft client sends a handshake packet as its first message. This packet contains the server address the player typed (or clicked) to connect. Infrarust reads that address and matches it against the `domains` list in each server configuration file.

```toml
# servers/survival.toml
domains = ["survival.mc.example.com"]
addresses = ["10.0.1.10:25565"]
```

A player connecting to `survival.mc.example.com` hits this config and gets routed to `10.0.1.10:25565`.

## Exact domains

Exact domains are stored in a hash map for O(1) lookup. They always take priority over wildcard patterns.

```toml
# servers/lobby.toml
domains = ["play.mc.example.com", "mc.example.com"]
addresses = ["10.0.1.1:25565"]
```

A single server can list multiple domains. All matching is case-insensitive: `Play.MC.Example.COM` resolves the same as `play.mc.example.com`.

## Wildcard patterns

Domains containing `*` or `?` are treated as wildcard patterns. They are compiled once at load time and tested sequentially against incoming connections.

`*` matches any number of characters (including none). `?` matches exactly one character.

```toml
# servers/catch-all.toml
domains = ["*.mc.example.com"]
addresses = ["10.0.1.1:25565"]
```

This matches `survival.mc.example.com`, `creative.mc.example.com`, and any other subdomain of `mc.example.com`.

### Pattern examples

| Pattern | Matches | Does not match |
|---------|---------|----------------|
| `*.mc.example.com` | `survival.mc.example.com`, `a.mc.example.com` | `mc.example.com` |
| `*.example.com` | `mc.example.com`, `play.example.com` | `example.com` |
| `play-?.mc.com` | `play-1.mc.com`, `play-a.mc.com` | `play-12.mc.com` |
| `*.com` | `anything.com`, `deep.sub.com` | `anything.net` |

### Priority rules

1. Exact domains always win over wildcards, regardless of insertion order.
2. Among wildcards, the first matching pattern wins. Put more specific patterns before broader ones.

```toml
# servers/survival.toml — loaded first
domains = ["*.survival.example.com"]
addresses = ["10.0.1.10:25565"]

# servers/catch-all.toml — loaded second
domains = ["*.example.com"]
addresses = ["10.0.1.1:25565"]
```

A player connecting to `play.survival.example.com` matches `*.survival.example.com` because that pattern was registered first. A player connecting to `creative.example.com` falls through to `*.example.com`.

::: warning
Wildcard ordering depends on the order server config files are loaded. Name your files so that more specific configs sort before broader ones (e.g., `01-survival.toml` before `99-catch-all.toml`), or use exact domains where possible.
:::

## Domain conflicts

If two server configs claim the same exact domain, the last one loaded wins. Infrarust logs a warning when this happens:

```
WARN domain conflict: overwriting previous provider
```

Avoid this by giving each server unique domains.

## FML markers

Forge and Fabric clients append markers to the hostname in the handshake packet: `\0FML\0`, `\0FML2\0`, or `\0FML3\0`. Infrarust strips these markers before domain resolution. You do not need to account for them in your `domains` list.

A player using a Forge client connecting to `mc.example.com` sends `mc.example.com\0FML2\0` in the handshake. The proxy strips the marker and resolves `mc.example.com` normally.

## SRV records

Minecraft clients support DNS SRV records for service discovery. An SRV record for `_minecraft._tcp.mc.example.com` tells the client which host and port to connect to. This happens on the client side, before any packet reaches Infrarust.

To use SRV records with Infrarust, point the SRV record at the proxy's address:

```
_minecraft._tcp.mc.example.com.  86400  IN  SRV  0 5 25565 proxy.example.com.
```

The player types `mc.example.com` in their server list. Their client resolves the SRV record and connects to `proxy.example.com:25565`. The handshake packet still contains `mc.example.com` as the server address, so Infrarust routes based on that domain.

::: tip
SRV records let players connect without typing a port number. If your proxy listens on the default port (25565), SRV records are optional but still useful for directing traffic through a CDN or load balancer.
:::

## Domain rewrite

By default, Infrarust forwards the original domain to the backend server. Some backends expect a specific hostname in the handshake. The `domain_rewrite` option controls what gets sent.

::: code-group

```toml [No rewrite (default)]
domains = ["play.mc.example.com"]
addresses = ["10.0.1.10:25565"]
domain_rewrite = "none"
```

```toml [Explicit domain]
domains = ["play.mc.example.com"]
addresses = ["10.0.1.10:25565"]
domain_rewrite = { explicit = "localhost" }
```

```toml [From backend address]
domains = ["play.mc.example.com"]
addresses = ["backend.local:25565"]
domain_rewrite = "from_backend"
```

:::

The three modes:

- `none` forwards the original domain unchanged. This is the default.
- `explicit` replaces the domain with a fixed string. Useful when the backend checks the hostname.
- `from_backend` rewrites the domain to the host part of the first backend address. In the example above, `play.mc.example.com` becomes `backend.local`.

## Unknown domains

When a player connects with a domain that matches no server config, the `unknown_domain_behavior` setting in `infrarust.toml` controls what happens:

```toml
# infrarust.toml
unknown_domain_behavior = "default_motd"
```

- `default_motd` (default) — responds with the global default MOTD for status pings, and rejects login attempts.
- `drop` — silently closes the connection.

## Servers without domains

A server config with an empty `domains` list is not reachable through domain routing. It can only be reached through server switching (transferring a player from one backend to another within the same network).

```toml
# servers/hub-backend.toml
name = "hub-backend"
network = "main"
domains = []
addresses = ["10.0.1.50:25565"]
```

This server is invisible to direct connections but available as a switch target for other servers in the `main` network.

## Full example

A typical setup with a lobby, two game servers, and a catch-all:

```toml
# servers/lobby.toml
name = "lobby"
network = "main"
domains = ["play.mc.example.com", "mc.example.com"]
addresses = ["10.0.1.1:25565"]
proxy_mode = "client_only"
```

```toml
# servers/survival.toml
name = "survival"
network = "main"
domains = ["survival.mc.example.com"]
addresses = ["10.0.1.10:25565"]
proxy_mode = "passthrough"
```

```toml
# servers/creative.toml
name = "creative"
network = "main"
domains = ["creative.mc.example.com"]
addresses = ["10.0.1.11:25565"]
proxy_mode = "client_only"
```

```toml
# servers/catch-all.toml
name = "catch-all"
network = "main"
domains = ["*.mc.example.com"]
addresses = ["10.0.1.1:25565"]
proxy_mode = "client_only"
```

Players connecting to `play.mc.example.com` or `mc.example.com` go to the lobby (exact match). `survival.mc.example.com` and `creative.mc.example.com` go to their servers (exact match). Any other subdomain of `mc.example.com` falls through to the catch-all, which also points at the lobby.
