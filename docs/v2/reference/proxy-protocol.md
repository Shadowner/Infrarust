---
title: Proxy Protocol
description: How Infrarust handles HAProxy proxy protocol v1 and v2 for preserving client IP addresses through load balancers and reverse proxies.
---

# Proxy Protocol

When Infrarust sits behind a load balancer or reverse proxy (HAProxy, nginx, AWS NLB), the TCP connection's source address becomes the load balancer's IP instead of the player's real IP. The HAProxy proxy protocol solves this by prepending the original client address to the connection before any application data.

Infrarust supports both proxy protocol v1 (text) and v2 (binary) on the receive side, and always sends v2 when forwarding to backends.

## Two separate settings

Proxy protocol has two independent settings that control different directions:

`receive_proxy_protocol` is a global setting in `infrarust.toml`. It tells the listener to expect a proxy protocol header from the upstream connection (your load balancer). Every incoming connection must include the header when this is enabled. It comes with `trusted_proxies`, the list of peers allowed to send that header.

`send_proxy_protocol` is a per-server setting. It tells Infrarust to prepend a v2 proxy protocol header when connecting to that backend. Enable this when your Minecraft server (or another proxy in front of it) expects proxy protocol.

## Receiving proxy protocol

Add `receive_proxy_protocol = true` and `trusted_proxies` to your `infrarust.toml`:

```toml{4-5}
bind = "0.0.0.0:25565"
max_connections = 1000

receive_proxy_protocol = true
trusted_proxies = ["10.0.0.5/32", "10.0.1.0/24"]
```

::: danger
When `receive_proxy_protocol` is enabled, every incoming connection must start with a valid proxy protocol header. Connections without a header (direct player connections, monitoring probes) will be rejected. Only enable this if all traffic comes through a proxy that sends the header.
:::

Once a proxy protocol header is decoded, the client's real IP and port are stored in the connection metadata. Any feature that uses the client address (rate limiting, the per-IP connection limit, IP filters, bans, logging) will see the original client IP rather than the load balancer's address.

### Trusted proxies

`trusted_proxies` is a list of IP addresses and CIDR ranges, IPv4 or IPv6, empty by default. A bare address is a single host: `10.0.0.5` is read as `10.0.0.5/32`, and `2001:db8::5` as `2001:db8::5/128`. An entry that is neither fails to parse, and the proxy refuses to start with `invalid trusted_proxies entry "<entry>": expected an IP address such as 192.168.1.10 or a CIDR range such as 10.0.0.0/8`. It is required when `receive_proxy_protocol` is `true`: with an empty list, configuration validation fails and the proxy does not start.

```
receive_proxy_protocol requires trusted_proxies: list the addresses of your load balancers, or ["0.0.0.0/0", "::/0"] to trust every peer
```

The listener compares the TCP peer address of each accepted connection with the list. A peer outside it is closed without a single byte of its header being read, and the close is logged at `debug` only (`closing connection from a peer outside trusted_proxies`), so a scan of the port does not fill the logs. An IPv4 peer that reaches a dual-stack socket as `::ffff:a.b.c.d` is compared as IPv4, so it matches IPv4 entries and not `::/0`.

`trusted_proxies = ["0.0.0.0/0", "::/0"]` accepts a header from every peer, which is what the proxy did before this option existed. With that value, any client that can reach the port chooses the address the proxy records for it.

### What gets parsed

The decoder reads up to 536 bytes from the start of the connection. It checks for the v2 binary signature first (the 12-byte magic), then falls back to the v1 text prefix (`PROXY `). Both IPv4 and IPv6 addresses are supported in either version. If neither signature matches after 16 bytes, the connection is closed with an error. The whole header must arrive within 5 seconds, otherwise the read times out and the connection is dropped.

The header is read in the connection's own task, after the listener has accepted it and before transport filters or the handshake parser run. A client that opens a connection and stays silent, or trickles its header in, delays only that connection, never the listener. While it waits for its header, the connection counts toward `max_connections`, and it gives its slot back when it is dropped. The `max_connections_per_ip` slot is taken after the header is decoded, so it is counted against the real client address and not against the load balancer.

A v1 `PROXY UNKNOWN` header is accepted (HAProxy sends it for health checks) but carries no address, so no real client IP is recorded for that connection, and it counts toward `max_connections_per_ip` under the load balancer's address. Address families other than TCP over IPv4 or IPv6, such as Unix sockets, are rejected.

## Sending proxy protocol

Add `send_proxy_protocol = true` to a server configuration file:

```toml{5}
domains = ["survival.mc.example.com"]
addresses = ["10.0.1.5:25565"]
proxy_mode = "passthrough"

send_proxy_protocol = true
```

When enabled, Infrarust writes a v2 binary header to the backend connection before sending any Minecraft packets. The header contains the player's real address: if a proxy protocol header was received on the incoming side, the original client IP is forwarded. Otherwise, the TCP peer address is used. This is independent of `proxy_mode`, so it works with any mode.

### Docker configuration

When using the Docker provider, set the label on your container:

```yaml
labels:
  infrarust.send_proxy_protocol: "true"
```

Only `true` or `1` turns it on. Any other value, or the absence of the label, leaves it off.

## Chaining proxies

If you run Infrarust behind a load balancer and in front of another proxy that also expects proxy protocol, enable both settings:

```
Player → HAProxy (sends PP) → Infrarust (receives + sends PP) → Backend
```

In `infrarust.toml`:

```toml
receive_proxy_protocol = true
trusted_proxies = ["10.0.0.5/32"]
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
| `InvalidProxyProtocol: unsupported address family in v2 header` | Address family other than TCP over IPv4 or IPv6 (for example, Unix sockets) |
| `ProxyProtocolDecode: proxy protocol header read timed out` | Header did not arrive within 5 seconds |
| `ProxyProtocolDecode` | Malformed header that matches a signature but fails to parse |

All proxy protocol errors are fatal. The connection is closed immediately with no fallback, and the error is logged at `warn` as a connection error.

A peer outside `trusted_proxies` is not in this table: no header is read, and the close is logged at `debug`.

## Backend compatibility

Your Minecraft server needs proxy protocol support to use `send_proxy_protocol`. BungeeCord and Velocity support it natively through their configuration. Paper does not support it directly, but you can place Paper behind Velocity with proxy protocol enabled. Another Infrarust instance can also receive proxy protocol from the first one, which is the chaining case described above.
