---
title: Quick Start
description: Set up Infrarust in five minutes. Create a minimal config, start the proxy, and connect with a Minecraft client.
---

# Quick Start

This guide walks you through a minimal Infrarust setup: one proxy, one backend Minecraft server, one player connecting through a domain.

## Interactive setup

The fastest way to get started is to run `infrarust` with no configuration file. The built-in setup wizard walks you through the essentials:

```bash
infrarust
```

When no `infrarust.toml` is found in the working directory, the wizard prompts you for:

- **Proxy listen address** - the address and port players connect to (default `0.0.0.0:25565`)
- **Servers directory** - where backend server definitions live (default `./servers`)
- **Maximum connections** - connection limit, `0` for unlimited
- **Web admin panel** - enable the REST API and web dashboard, choose a bind address and port
- **Sample server** - optionally create a first backend server definition with a domain and address

If you enable the web panel, the wizard generates an API key and writes it to `plugins/admin_api/config.toml`. Save this key, it is required to access the dashboard.

Once confirmed, the wizard writes `infrarust.toml`, creates the servers directory, and starts the proxy immediately.

::: tip
If you prefer to write the configuration by hand, skip this section and continue with the [manual setup below](#prerequisites). For the full list of options, see the [configuration reference](../reference/config-schema).
:::

## Prerequisites

- Infrarust installed ([Installation](./installation.md))
- A Minecraft Java Edition server running somewhere you can reach (local machine, LAN, remote host)
- A domain name pointing to the machine running Infrarust, or `localhost` for local testing

## 1. Create the proxy config

Infrarust reads its main configuration from `infrarust.toml` in the working directory. Create the file with the minimum required settings:

```toml
bind = "0.0.0.0:25565"
servers_dir = "./servers"

[web]
```

`bind` sets the address and port Infrarust listens on. `servers_dir` tells it where to find server definitions. The `[web]` section enables the admin REST API and web dashboard on port `8080`.

::: tip
`bind` and `servers_dir` are the defaults. The `[web]` section activates the [Admin API & Web UI](../plugins/builtin/admin-api) plugin with all defaults - API and dashboard on `http://127.0.0.1:8080`.
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
INFO starting infrarust v2.0.0-alpha.4
     bind=0.0.0.0:25565 servers_dir=./servers
INFO Generated admin API key: a1b2c3d4-e5f6-...
INFO Admin API server starting bind=127.0.0.1:8080
INFO infrarust is ready, accepting connections
```

The admin API key is written to `plugins/admin_api/config.toml`. Open `http://127.0.0.1:8080` in a browser to access the web dashboard.

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

[web]
```

Run the container, exposing both the Minecraft port and the web dashboard:

```bash
docker run -d \
  --name infrarust \
  -p 25565:25565 \
  -p 8080:8080 \
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
- [Admin API & Web UI](../plugins/builtin/admin-api) for the full REST API reference and SSE streaming
