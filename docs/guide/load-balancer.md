---
title: Running Behind a Load Balancer
description: How to deploy Infrarust behind HAProxy, nginx, or a cloud load balancer with proxy protocol and TCP keepalive tuning.
---

# Running Behind a Load Balancer

Infrarust works behind any TCP load balancer. Minecraft uses raw TCP, so your load balancer must operate at layer 4 (TCP), not layer 7 (HTTP). This page covers proxy protocol setup, keepalive tuning, and the mistakes that trip people up most often.

## Architecture

A typical deployment looks like this:

```
Players → Load Balancer (TCP, port 25565) → Infrarust → Backend Servers
```

The load balancer distributes TCP connections to one or more Infrarust instances. Each Infrarust instance routes players to the correct backend based on the domain they connected with.

## Proxy Protocol

When a load balancer sits in front of Infrarust, the source IP of every connection becomes the load balancer's IP instead of the player's real IP. Proxy protocol fixes this. The load balancer prepends a small header to each TCP connection that contains the original client address.

Infrarust supports both proxy protocol v1 (text) and v2 (binary).

### Receiving Proxy Protocol

To accept proxy protocol headers from your load balancer, set `receive_proxy_protocol` in your global config:

```toml{2}
# infrarust.toml
receive_proxy_protocol = true
```

::: danger
Only enable `receive_proxy_protocol` if your load balancer actually sends proxy protocol headers. If you enable it without a load balancer in front, clients can forge their source IP by sending a fake proxy protocol header.
:::

### Sending Proxy Protocol to Backends

If your backend Minecraft servers also need to know the real client IP (for ban lists, logging, or plugins), Infrarust can forward proxy protocol v2 headers to them. Set this per server:

```toml{3}
# servers/survival.toml
addresses = ["backend:25565"]
send_proxy_protocol = true
```

The backend server must support proxy protocol. For vanilla Minecraft, you'll need a plugin like [HAProxyDetector](https://github.com/andylizi/haproxy-detector) or a modded server that understands proxy protocol.

### HAProxy Configuration

Configure HAProxy to forward TCP traffic with proxy protocol v2:

```haproxy
frontend minecraft
    bind *:25565
    mode tcp
    default_backend infrarust

backend infrarust
    mode tcp
    server infrarust1 10.0.0.10:25565 send-proxy-v2
    server infrarust2 10.0.0.11:25565 send-proxy-v2
```

Then enable `receive_proxy_protocol = true` in Infrarust.

### nginx Configuration

nginx supports proxy protocol through its stream module:

```nginx
stream {
    upstream infrarust {
        server 10.0.0.10:25565;
        server 10.0.0.11:25565;
    }

    server {
        listen 25565;
        proxy_pass infrarust;
        proxy_protocol on;
    }
}
```

### Cloud Load Balancers

Most cloud providers offer TCP load balancers with proxy protocol support:

- **AWS NLB:** Enable proxy protocol v2 on the target group.
- **GCP TCP Load Balancer:** Enable PROXY protocol in the backend service configuration.
- **Azure Load Balancer:** Azure's standard load balancer does not support proxy protocol. Use HAProxy or nginx on a VM instead.

## TCP Keepalive

Load balancers drop idle TCP connections after a timeout (typically 60-350 seconds depending on the provider). Minecraft players sitting in a server selection screen or AFK can trigger this. TCP keepalive probes prevent the load balancer from closing connections that are still alive.

Infrarust applies keepalive settings to both incoming player connections and outgoing backend connections. Configure them in the global config:

```toml
# infrarust.toml
[keepalive]
time = "30s"
interval = "10s"
retries = 3
```

| Option | Default | Description |
|--------|---------|-------------|
| `time` | `30s` | Idle time before the first keepalive probe |
| `interval` | `10s` | Time between subsequent probes |
| `retries` | `3` | Failed probes before the connection is closed |

::: tip
Set `time` lower than your load balancer's idle timeout. AWS NLB defaults to 350 seconds, GCP to 600 seconds. The default of 30 seconds works for all major providers.
:::

::: info
The `retries` field only takes effect on Linux and macOS. On Windows, the OS controls the retry count.
:::

## SO_REUSEPORT

If you run multiple Infrarust instances on the same machine, enable `so_reuseport` so they can all bind to the same port. The kernel distributes incoming connections across instances:

```toml{3}
# infrarust.toml
bind = "0.0.0.0:25565"
so_reuseport = true
```

This only works on Linux.

## Connection Limits

To prevent a single Infrarust instance from accepting more connections than it can handle, set `max_connections`:

```toml
# infrarust.toml
max_connections = 10000
```

A value of `0` (the default) means no limit.

## Common Pitfalls

### Proxy protocol mismatch

If the load balancer sends proxy protocol but Infrarust doesn't expect it, connections fail immediately. The reverse is also true: if Infrarust expects proxy protocol but the load balancer doesn't send it, Infrarust tries to parse the Minecraft handshake as a proxy protocol header and drops the connection.

Both sides must agree. Enable proxy protocol on both the load balancer and Infrarust, or on neither.

### Using an HTTP load balancer

Minecraft is not HTTP. If you configure your load balancer in HTTP/layer 7 mode, it will try to parse the Minecraft protocol as HTTP and reject every connection. Always use TCP/layer 4 mode.

### Idle timeout too low

Some cloud load balancers have aggressive idle timeouts (AWS ALB: 60 seconds). If players get disconnected while AFK or in a lobby, the load balancer is probably closing the connection. Lower the `keepalive.time` value to send probes more frequently, or increase the load balancer's idle timeout.

### Health checks hitting Infrarust

Load balancers send health check probes to verify backends are alive. Infrarust expects Minecraft protocol traffic, so HTTP health checks will fail. Configure your load balancer to use TCP health checks (just verify the port is open) instead of HTTP health checks.

### Forgetting `send_proxy_protocol` on the backend

If you enable `receive_proxy_protocol` on Infrarust but forget to set `send_proxy_protocol = true` in the server config, your backend Minecraft server will see Infrarust's IP instead of the player's IP. The proxy protocol chain must be complete from load balancer to Infrarust to backend.
