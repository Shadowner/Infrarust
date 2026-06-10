---
title: IP Filtering
description: Restrict which IP addresses can connect to your Infrarust proxy using whitelist and blacklist rules with CIDR notation.
---

# IP Filtering

Infrarust can restrict connections based on client IP address. You define rules using CIDR notation, and the proxy checks every incoming connection against them before forwarding traffic to a backend server.

Two modes are available: whitelist (only listed IPs may connect) and blacklist (listed IPs are blocked). Both can be set at the same time. Filters can be configured globally and per server.

## How it works

The filter evaluates rules in this order:

1. If the IP matches any blacklist entry, it is rejected immediately.
2. If the whitelist is non-empty and the IP does not match any whitelist entry, it is rejected.
3. Otherwise the connection is allowed.

This means the blacklist always wins. A blacklisted IP inside a whitelisted range is still rejected. If you only need a whitelist, leave the blacklist empty; if you only need a blacklist, leave the whitelist empty.

## Configuration

`ip_filter` can be placed in the global `infrarust.toml` or in any server configuration file. A connection must pass both the global filter and the per-server filter.

### Global filter

```toml
[ip_filter]
whitelist = ["192.168.1.0/24"]
```

### Per-server filter

Add an `ip_filter` section to a server configuration file:

```toml
# Allow only your local network
[ip_filter]
whitelist = ["192.168.1.0/24", "10.0.0.0/8"]
```

```toml
# Block a specific subnet
[ip_filter]
blacklist = ["10.0.99.0/24"]
```

Both fields accept a list of CIDR ranges. Single IPs use `/32` for IPv4 or `/128` for IPv6:

```toml
[ip_filter]
whitelist = ["203.0.113.42/32", "2001:db8::1/128"]
```

## CIDR notation

CIDR ranges define a block of IP addresses. The number after the slash is the prefix length, which determines how many addresses the range covers.

| CIDR | Matches | Example range |
|------|---------|---------------|
| `192.168.1.0/24` | 256 addresses | 192.168.1.0 to 192.168.1.255 |
| `10.0.0.0/8` | 16.7M addresses | 10.0.0.0 to 10.255.255.255 |
| `203.0.113.42/32` | 1 address | 203.0.113.42 only |

## Proxy protocol

If your proxy sits behind a load balancer or DDoS protection service that uses HAProxy proxy protocol (v1 or v2), Infrarust extracts the real client IP from the proxy protocol header. IP filters apply to the real client IP, not the load balancer's address.

## Pipeline position

IP filtering runs at two points in the connection pipeline. The global filter runs first, before ban checks, handshake parsing, and rate limiting. The per-server filter runs inside the domain router, after the incoming domain has been matched to a server config.

A connection must pass both filters. The global filter rejects unwanted IPs before any packet is parsed; per-server filters let you restrict individual backends independently.

## Examples

### Private server with a whitelist

Only allow connections from a home network and a friend's IP:

```toml
[ip_filter]
whitelist = ["192.168.1.0/24", "203.0.113.50/32"]
```

Any IP outside these ranges is rejected with "IP 45.33.22.11 is not allowed on this server" (per-server filter) or "IP 45.33.22.11 is not allowed" (global filter).

### Public server with a blocklist

Block a known bad subnet while allowing everyone else:

```toml
[ip_filter]
blacklist = ["198.51.100.0/24", "10.0.99.0/24"]
```

### Combining a whitelist and a blacklist

Because the blacklist is evaluated before the whitelist, you can carve out exceptions inside a broader whitelist. For example, allow `10.0.0.0/8` but block a specific range within it:

```toml
[ip_filter]
whitelist = ["10.0.0.0/8"]
blacklist = ["10.6.6.0/24"]
```

IPs in `10.6.6.0/24` are rejected even though they fall inside the whitelisted `/8`.

### Combining with other security features

IP filtering works alongside [rate limiting](./rate-limiting) and [bans](./bans). A connection passes through IP filtering first, then the ban check, then rate limiting, then domain routing.
