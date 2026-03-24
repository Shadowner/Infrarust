---
title: What is Infrarust?
description: Infrarust is a Minecraft reverse proxy written in Rust. It routes players to backend servers based on the domain they connect with.
---

# What is Infrarust?

Infrarust is a reverse proxy for Minecraft Java Edition. It sits between your players and your Minecraft servers, routing connections based on the domain a player uses to connect.

You run one Infrarust instance on a single IP address. Players connect with different domains (e.g. `survival.mc.example.com`, `creative.mc.example.com`), and Infrarust forwards each connection to the right backend server. Your servers can run on different machines, different ports, or inside Docker containers.

## Why use a reverse proxy?

Without a proxy, every Minecraft server needs its own public IP or a unique port. Players have to remember `play.example.com:25566` for one server and `play.example.com:25567` for another.

With Infrarust, all servers share a single address on port 25565. Players connect to `survival.mc.example.com` or `creative.mc.example.com` and Infrarust handles the rest. You get:

- One IP address for all your servers
- Domain-based routing with wildcard support (`*.mc.example.com`)
- Configuration changes without restarting the proxy
- Per-server proxy modes that control how traffic is handled
- A plugin system for authentication, queuing, and server wake-on-connect

## Who is Infrarust for?

Infrarust is built for Minecraft server administrators who run multiple backend servers and want a single entry point for players. If you run a network with a hub, survival, creative, and minigames servers, Infrarust replaces the need for separate IPs or port numbers.

You should be comfortable with basic server administration: editing TOML config files, running Docker containers, or managing systemd services. You do not need to know Rust.

## How it works

When a Minecraft client connects, it sends a handshake packet that includes the server address the player typed. Infrarust reads that address, looks it up in its domain index, and opens a connection to the matching backend server.

Each backend server is defined in a TOML file inside a `servers/` directory:

```toml
domains = ["survival.mc.example.com"]
addresses = ["10.0.1.10:25565"]
proxy_mode = "passthrough"
```

Infrarust watches this directory for changes. Add, edit, or remove a server file and the routing table updates without a restart.

## Proxy modes

Infrarust supports six proxy modes. Each mode controls how much the proxy inspects or modifies the traffic between client and server:

| Mode | What it does |
|------|-------------|
| `passthrough` | Forwards raw bytes after the handshake. Default mode. |
| `zerocopy` | Uses Linux `splice(2)` for kernel-level forwarding. Linux only. |
| `client_only` | Handles Mojang authentication on the proxy side. Backend runs `online_mode=false`. |
| `offline` | No authentication. Transparent relay. |
| `server_only` | Authentication handled by the backend server. |
| `full` | Encryption on both the client and server sides. |

You set the mode per server in its config file. Pick `passthrough` unless you need the proxy to handle authentication or packet inspection.

## Key features

**Domain routing** matches players to servers by the hostname they connect with. Exact domains resolve through a hash map lookup. Wildcard patterns like `*.mc.example.com` are pre-compiled when the config loads.

**Server discovery** works through two providers. The file provider watches your `servers/` directory for TOML config changes. The Docker provider reads container labels from the Docker socket and watches for container start/stop events.

**Plugins** extend the proxy without modifying its source. Built-in plugins handle Mojang authentication, player queuing when a server is full, and starting stopped servers when a player connects. You can write your own plugins against the `infrarust-api` crate.

**Security** includes per-IP rate limiting, a ban system with expiry and audit logging, IP allow/deny lists per server, and HAProxy proxy protocol support for preserving client IPs behind load balancers.

**Telemetry** exports traces and metrics via OpenTelemetry to any OTLP-compatible backend (Jaeger, Grafana Tempo, etc.).

## Next steps

Ready to set up Infrarust? Head to the [Quick Start](./quick-start.md) guide to get a working proxy in a few minutes.
