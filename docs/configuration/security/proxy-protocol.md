---
title: Proxy Protocol
description: Configure HAProxy proxy protocol to preserve real client IPs through load balancers and reverse proxies.
---

# Proxy Protocol

When Infrarust sits behind a load balancer or reverse proxy (like HAProxy, nginx, or a cloud LB), the proxy's IP replaces the player's real IP on every connection. Proxy protocol solves this by prepending a small header to each TCP connection that carries the original client address.

Infrarust supports both receiving proxy protocol headers from an upstream proxy and sending them to backend Minecraft servers.

## Receiving proxy protocol

Set `receive_proxy_protocol` in your `infrarust.toml` to decode incoming proxy protocol headers:

```toml{3}
[proxy]
bind = "0.0.0.0:25565"
receive_proxy_protocol = true
```

When enabled, Infrarust expects every incoming connection to start with a valid proxy protocol header. It auto-detects whether the header uses v1 (text) or v2 (binary) format.

::: danger
This is an all-or-nothing setting. Once enabled, connections without a proxy protocol header will be rejected. Only enable this if all traffic comes through a proxy that sends these headers.
:::

The decoded client IP becomes the "real IP" for the connection. Infrarust uses it for:

- IP-based rate limiting
- IP filtering (allow/deny lists)
- Ban enforcement
- Logging

## Sending proxy protocol to backends

Per-server, you can forward the client's real address to the backend Minecraft server. Set `send_proxy_protocol` in the server config file:

```toml{4}
domains = ["survival.example.com"]
addresses = ["10.0.1.5:25565"]
proxy_mode = "client_only"
send_proxy_protocol = true
```

Infrarust always sends v2 (binary) headers to backends. If the connection arrived with a proxy protocol header, Infrarust forwards the original client address. Otherwise, it sends the direct peer address.

::: warning
Your backend Minecraft server must support proxy protocol. For Paper/Purpur servers, enable `proxy-protocol: true` in `paper-global.yml` under `proxies > velocity`. Vanilla servers do not support proxy protocol.
:::

### Docker labels

When using the Docker provider, set the label on your container:

```yaml
labels:
  infrarust.send_proxy_protocol: "true"
```

## V1 vs V2

| | V1 (text) | V2 (binary) |
|---|---|---|
| Format | Human-readable ASCII line | Binary with 12-byte signature |
| IPv4 | Yes | Yes |
| IPv6 | Yes | Yes |
| Receiving | Auto-detected | Auto-detected |
| Sending | Not used | Always used |

Infrarust auto-detects v1 and v2 when receiving. When sending to backends, it always uses v2. You do not need to configure which version to use.

## Typical deployment

A common setup puts a cloud load balancer or HAProxy in front of Infrarust, which then connects to backend Minecraft servers:

```
Player → Load Balancer (adds PP header) → Infrarust (decodes PP) → Backend MC Server
                                           receive_proxy_protocol     send_proxy_protocol
```

The `infrarust.toml` config for this scenario:

```toml
[proxy]
bind = "0.0.0.0:25565"
receive_proxy_protocol = true
```

And each server config that needs the real IP forwarded:

```toml
domains = ["mc.example.com"]
addresses = ["10.0.1.10:25565"]
send_proxy_protocol = true
```

## Configuration reference

| Option | File | Type | Default | Description |
|---|---|---|---|---|
| `receive_proxy_protocol` | `infrarust.toml` | `bool` | `false` | Decode proxy protocol headers on incoming connections |
| `send_proxy_protocol` | server config | `bool` | `false` | Send a v2 proxy protocol header to the backend |
