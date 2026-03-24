---
title: Proxy Protocol
description: How Infrarust handles HAProxy proxy protocol v1 and v2 for preserving client IP addresses through load balancers and reverse proxies.
---

# Proxy Protocol

When Infrarust sits behind a load balancer or reverse proxy (HAProxy, nginx, AWS NLB), the TCP connection's source address becomes the load balancer's IP instead of the player's real IP. The HAProxy proxy protocol solves this by prepending the original client address to the connection before any application data.

Infrarust supports both proxy protocol v1 (text) and v2 (binary) on the receive side, and always sends v2 when forwarding to backends.

## Two separate settings

Proxy protocol has two independent settings that control different directions:

`receive_proxy_protocol` is a global setting in `infrarust.toml`. It tells the listener to expect a proxy protocol header from the upstream connection (your load balancer). Every incoming connection must include the header when this is enabled.

`send_proxy_protocol` is a per-server setting. It tells Infrarust to prepend a v2 proxy protocol header when connecting to that backend. Enable this when your Minecraft server (or another proxy in front of it) expects proxy protocol.

## Receiving proxy protocol

Add `receive_proxy_protocol = true` to your `infrarust.toml`:

```toml{4}
bind = "0.0.0.0:25565"
max_connections = 1000

receive_proxy_protocol = true
```

::: danger
When `receive_proxy_protocol` is enabled, every incoming connection must start with a valid proxy protocol header. Connections without a header (direct player connections, monitoring probes) will be rejected. Only enable this if all traffic comes through a proxy that sends the header.
:::

Once a proxy protocol header is decoded, the client's real IP and port are stored in the connection metadata. Any feature that uses the client address (rate limiting, IP filters, bans, logging) will see the original client IP rather than the load balancer's address.

### What gets parsed

The decoder reads up to 536 bytes from the start of the connection. It checks for the v2 binary signature first (12-byte magic), then falls back to the v1 text prefix (`PROXY `). Both IPv4 and IPv6 addresses are supported in either version. If neither signature matches after 16 bytes, the connection is closed with an error.

## Sending proxy protocol

Add `send_proxy_protocol = true` to a server configuration file:

```toml{5}
domains = ["survival.mc.example.com"]
addresses = ["10.0.1.5:25565"]
proxy_mode = "Intercepted"

send_proxy_protocol = true
```

When enabled, Infrarust writes a v2 binary header to the backend connection before sending any Minecraft packets. The header contains the player's real address: if a proxy protocol header was received on the incoming side, the original client IP is forwarded. Otherwise, the TCP peer address is used.

### Docker configuration

When using the Docker provider, set the label on your container:

```yaml
labels:
  infrarust.send_proxy_protocol: "true"
```

The label accepts `true`, `1`, `false`, or `0`. It defaults to `false`.

## Chaining proxies

If you run Infrarust behind a load balancer and in front of another proxy that also expects proxy protocol, enable both settings:

```
Player → HAProxy (sends PP) → Infrarust (receives + sends PP) → Backend
```

In `infrarust.toml`:

```toml
receive_proxy_protocol = true
```

In `servers/backend.toml`:

```toml
send_proxy_protocol = true
```

Infrarust preserves the original client address through the chain. The v2 header sent to the backend contains the IP from the received proxy protocol header, not Infrarust's own address.

### Mixed IP versions

When the source and destination addresses use different IP versions (one IPv4, one IPv6), the encoder maps IPv4 addresses to IPv6 using the standard `::ffff:0:0/96` mapping. The backend sees an IPv6-mapped address in this case.

## Error handling

| Error | Cause |
|-------|-------|
| `InvalidProxyProtocol: header exceeds maximum size` | Header larger than 536 bytes |
| `InvalidProxyProtocol: connection closed before proxy protocol header` | Connection dropped before header was complete |
| `InvalidProxyProtocol: data does not start with a proxy protocol signature` | No v1 or v2 signature found in the first 16 bytes |
| `InvalidProxyProtocol: unsupported address family` | Address family other than IPv4/IPv6 (e.g., Unix sockets) |
| `ProxyProtocolDecode` | Malformed header that matches a signature but fails to parse |

All proxy protocol errors are fatal. The connection is closed immediately with no fallback.

## Backend compatibility

Your Minecraft server needs proxy protocol support to use `send_proxy_protocol`. Some options:

- **BungeeCord/Velocity** support proxy protocol natively through their configuration
- **Paper** does not support proxy protocol directly, but you can place it behind Velocity with proxy protocol enabled
- Another Infrarust instance can receive proxy protocol from the first one
