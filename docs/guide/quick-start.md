---
title: Quick Start
description: Set up Infrarust in five minutes. Create a minimal config, start the proxy, and connect with a Minecraft client.
---

# Quick Start

This guide walks you through a minimal Infrarust setup: one proxy, one backend Minecraft server, one player connecting through a domain.

## Prerequisites

- Infrarust installed ([Installation](./installation.md))
- A Minecraft Java Edition server running somewhere you can reach (local machine, LAN, remote host)
- A domain name pointing to the machine running Infrarust, or `localhost` for local testing

## 1. Create the proxy config

Infrarust reads its main configuration from `infrarust.toml` in the working directory. Create the file with the minimum required settings:

```toml
bind = "0.0.0.0:25565"
servers_dir = "./servers"
```

`bind` sets the address and port Infrarust listens on. `servers_dir` tells it where to find server definitions.

::: tip
These are the defaults. An empty `infrarust.toml` file works too, but writing them out makes the setup explicit.
:::

## 2. Define a backend server

Create the `servers/` directory and add a TOML file for your backend. The filename can be anything ending in `.toml`.

```bash
mkdir servers
```

Create `servers/survival.toml`:

```toml
domains = ["survival.example.com"]
addresses = ["127.0.0.1:25566"]
```

`domains` lists the hostnames that route to this server. When a player connects to `survival.example.com`, Infrarust forwards the connection to `127.0.0.1:25566`.

`addresses` takes one or more `host:port` strings. If you omit the port, it defaults to `25565`.

::: info
For local testing without a real domain, you can add `127.0.0.1 survival.example.com` to your system's hosts file (`/etc/hosts` on Linux/macOS, `C:\Windows\System32\drivers\etc\hosts` on Windows).
:::

## 3. Start Infrarust

Run the binary from the directory containing `infrarust.toml`:

```bash
infrarust
```

You should see output like:

```
INFO starting infrarust v2.0.0-alpha.1
     bind=0.0.0.0:25565 servers_dir=./servers
INFO infrarust is ready, accepting connections
```

To use a config file at a different path:

```bash
infrarust --config /path/to/infrarust.toml
```

## 4. Connect with Minecraft

Open Minecraft Java Edition and add a server:

1. Go to **Multiplayer** > **Add Server**
2. Set the server address to `survival.example.com`
3. Click **Done**, then join

Infrarust reads the domain from the handshake packet and routes you to the backend at `127.0.0.1:25566`.

## Docker setup

If you prefer Docker, create a `config/` directory with your `infrarust.toml` and a `servers/` subdirectory inside it:

```
config/
├── infrarust.toml
└── servers/
    └── survival.toml
```

Set `servers_dir` in your `infrarust.toml` to match the container path:

```toml
bind = "0.0.0.0:25565"
servers_dir = "/app/config/servers"
```

Run the container:

```bash
docker run -d \
  --name infrarust \
  -p 25565:25565 \
  -v ./config:/app/config \
  ghcr.io/shadowner/infrarust:latest \
  --config /app/config/infrarust.toml
```

## Adding more servers

Drop another `.toml` file in the `servers/` directory. Infrarust watches the directory and picks up changes without a restart.

`servers/creative.toml`:

```toml
domains = ["creative.example.com"]
addresses = ["127.0.0.1:25567"]
proxy_mode = "client_only"
```

The `proxy_mode` field controls how Infrarust handles traffic. The default is `passthrough`, which forwards raw bytes after the handshake. `client_only` makes the proxy handle Mojang authentication so the backend can run with `online-mode=false`. See [Proxy Modes](../configuration/proxy-modes/) for the full list.

## What to read next

- [Configuration overview](../configuration/) for all global and per-server options
- [Proxy Modes](../configuration/proxy-modes/) to understand `passthrough`, `client_only`, `offline`, and the others
- [Server definitions](../configuration/servers.md) for the full set of per-server fields
