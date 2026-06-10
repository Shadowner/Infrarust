---
title: What is Infrarust?
description: Infrarust is a Minecraft reverse proxy written in Rust. It routes players to backend servers by domain, with five proxy modes, native and WASM plugins, limbo, server auto start/stop, OpenTelemetry, a web admin API, and built-in security controls.
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

Infrarust supports five proxy modes. Each mode controls how much the proxy inspects or modifies the traffic between client and server:

| Mode | What it does |
|------|-------------|
| `passthrough` | Forwards raw bytes after the handshake. Default mode. |
| `zero_copy` | Uses Linux `splice(2)` for kernel-level forwarding. Linux only. |
| `client_only` | Handles Mojang authentication on the proxy side. Backend runs `online-mode=false`. |
| `offline` | No authentication. Transparent relay. |
| `server_only` | Forwards raw bytes; authentication is handled by the backend. |

The first two modes (`passthrough`, `zero_copy`) and `server_only` are the forwarding family: they relay raw bytes after the handshake with no packet inspection. The remaining two (`client_only`, `offline`) are the intercepted family: the proxy parses packets, which enables server switching and limbo.

You set the mode per server in its config file.

## Key features

### Domain routing

Players are matched to servers by the hostname they connect with. Exact domains resolve through a hash map lookup. Wildcard patterns like `*.mc.example.com` are pre-compiled when the config loads.

### Server discovery

Two providers register backends. The file provider watches your `servers/` directory for TOML config changes. The Docker provider reads container labels from the Docker socket and watches for container start/stop events.

### Server auto start/stop

Infrarust can launch a backend when a player connects and shut it down when idle. The `[server_manager]` table in a server config supports local processes, Pterodactyl, and Crafty controllers. Players are held in limbo while the server boots.

### Limbo

Limbo is a proxy-side virtual world that holds players between connection states. Plugins use it to show a login screen, display a queue position, or keep players connected while a backend server restarts. Players in limbo see a void world and receive chat and title messages from the proxy.

### Plugins

Plugins extend the proxy without modifying its source. Native plugins are Rust crates compiled into the proxy binary and can access every proxy service, including transport-level packet filters. WASM plugins are sandboxed WebAssembly components loaded at runtime from the `plugins/` directory; they run with CPU and memory limits and can be written in any language targeting `wasm32-wasip2`. Built-in plugins cover Mojang authentication, server wake-on-connect, and player queuing.

### Web admin API and UI

A built-in REST API ships with an embedded dashboard. Add `[web]` to `infrarust.toml` to enable it. The dashboard shows connected players, server status, and logs, and the API supports real-time event streaming over SSE.

### BungeeCord and Velocity forwarding

Forwarding passes real player IP addresses and UUIDs to backend servers. Configure the `[forwarding]` table globally in `infrarust.toml` with `mode = "bungee_cord"` (alias `legacy`), `mode = "velocity"` (alias `modern`), or `mode = "bungee_guard"`.

### Security

Security controls include per-IP rate limiting, a ban system with expiry and audit logging (file `bans.json`), IP allow/deny lists per server or globally, and HAProxy proxy protocol support for preserving client IPs behind load balancers.

### Telemetry

Infrarust exports traces and metrics via OpenTelemetry OTLP to any compatible backend (Jaeger, Tempo, and similar). Configure the `[telemetry]` table; the default endpoint is `http://localhost:4317`.

## Next steps

Ready to set up Infrarust? Head to the [Quick Start](./quick-start) guide to get a working proxy in a few minutes.
