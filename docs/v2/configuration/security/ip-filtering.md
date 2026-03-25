---
title: IP Filtering
description: Restrict which IP addresses can connect to your Infrarust proxy using whitelist and blacklist rules with CIDR notation.
---

# IP Filtering

Infrarust can restrict connections based on client IP address. You define rules using CIDR notation, and the proxy checks every incoming connection against them before forwarding traffic to a backend server.

Two modes are available: **whitelist** (only listed IPs may connect) and **blacklist** (listed IPs are blocked). You can configure filters per server.

## How it works

The filter evaluates rules in this order:

1. If the whitelist is non-empty, only IPs matching a whitelist entry are allowed. The blacklist is ignored entirely.
2. If the whitelist is empty and the blacklist is non-empty, IPs matching a blacklist entry are rejected.
3. If both lists are empty, all IPs are allowed.

::: warning
Whitelist and blacklist are mutually exclusive in practice. When you set a whitelist, the blacklist has no effect. Pick one approach per server.
:::

## Per-server configuration

Add an `ip_filter` section to your server configuration file:

```yaml{2-4}
# Allow only your local network
ip_filter:
  whitelist:
    - "192.168.1.0/24"
    - "10.0.0.0/8"
```

```yaml{2-4}
# Block a specific subnet
ip_filter:
  blacklist:
    - "10.0.99.0/24"
```

Both fields accept a list of CIDR ranges. Single IPs use `/32` for IPv4 or `/128` for IPv6:

```yaml
ip_filter:
  whitelist:
    - "203.0.113.42/32"
    - "2001:db8::1/128"
```

## CIDR notation

CIDR ranges define a block of IP addresses. The number after the slash is the prefix length, which determines how many addresses the range covers.

| CIDR | Matches | Example range |
|------|---------|---------------|
| `192.168.1.0/24` | 256 addresses | 192.168.1.0 – 192.168.1.255 |
| `10.0.0.0/8` | 16.7M addresses | 10.0.0.0 – 10.255.255.255 |
| `203.0.113.42/32` | 1 address | 203.0.113.42 only |

## Proxy protocol

If your proxy sits behind a load balancer or DDoS protection service that uses HAProxy proxy protocol (v1 or v2), Infrarust extracts the real client IP from the proxy protocol header. IP filters apply to the real client IP, not the load balancer's address.

## Evaluation order in the pipeline

IP filtering runs at two points in the connection pipeline:

1. **Global filter** — checked before handshake parsing, blocks the connection immediately.
2. **Per-server filter** — checked after domain routing resolves which server the player is connecting to.

A connection must pass both filters. The global filter catches unwanted IPs early, and per-server filters let you restrict access to specific backends.

## Examples

### Private server with a whitelist

Only allow connections from a home network and a friend's IP:

```yaml
ip_filter:
  whitelist:
    - "192.168.1.0/24"
    - "203.0.113.50/32"
```

Any IP outside these ranges gets disconnected with a message like "IP 45.33.22.11 is not allowed on this server".

### Public server with a blacklist

Block a known bad subnet while allowing everyone else:

```yaml
ip_filter:
  blacklist:
    - "198.51.100.0/24"
    - "10.0.99.0/24"
```

### Combining with other security features

IP filtering works alongside [rate limiting](./rate-limiting.md) and [bans](./bans.md). A connection passes through IP filtering first, then rate limiting, then domain routing. Stack these features to build the level of protection you need.
