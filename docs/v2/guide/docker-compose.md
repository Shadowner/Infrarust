---
title: Docker Compose
description: Run Infrarust alongside Minecraft servers with Docker Compose, using container labels for automatic server discovery.
---

# Docker Compose

This page walks through a complete `docker-compose.yml` that runs Infrarust as a reverse proxy in front of multiple Minecraft servers. The servers register themselves through Docker labels, so you don't need static config files for each backend.

## Prerequisites

You need Docker and Docker Compose v2 installed. The examples use the `itzg/minecraft-server` image for backend servers, but any Minecraft server image works.

## Project layout

```
my-network/
├── docker-compose.yml
└── config/
    └── infrarust.toml
```

The `config/` directory is mounted into the Infrarust container. If you also want file-based server definitions, add a `servers/` subdirectory there.

## The infrarust.toml

A minimal config that enables Docker discovery:

```toml
bind = "0.0.0.0:25565"

[docker]
network = "minecraft"
```

The `[docker]` section activates the Docker provider. Setting `network = "minecraft"` tells Infrarust to resolve container IPs from that specific Docker network. Without it, Infrarust picks the first available network IP from each container, which can be wrong if containers are attached to multiple networks.

## Full docker-compose.yml

```yaml
networks:
  minecraft:
    driver: bridge

services:
  # ── Infrarust reverse proxy ──────────────────────
  infrarust:
    image: ghcr.io/shadowner/infrarust:latest
    container_name: infrarust
    command: ["--config", "/app/config/infrarust.toml"]
    ports:
      - "25565:25565"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro # [!code focus]
      - ./config:/app/config
    networks:
      - minecraft
    restart: unless-stopped

  # ── Lobby server ─────────────────────────────────
  lobby:
    image: itzg/minecraft-server
    container_name: lobby
    environment:
      EULA: "TRUE"
      TYPE: "PAPER"
      MEMORY: "1G"
    labels: # [!code focus]
      infrarust.enable: "true" # [!code focus]
      infrarust.domains: "mc.example.com" # [!code focus]
      infrarust.proxy_mode: "client_only" # [!code focus]
      infrarust.network: "main" # [!code focus]
    networks:
      - minecraft
    restart: unless-stopped

  # ── Survival server ──────────────────────────────
  survival:
    image: itzg/minecraft-server
    container_name: survival
    environment:
      EULA: "TRUE"
      TYPE: "PAPER"
      DIFFICULTY: "hard"
      MEMORY: "2G"
    labels:
      infrarust.enable: "true"
      infrarust.domains: "survival.mc.example.com"
      infrarust.proxy_mode: "client_only"
      infrarust.network: "main"
      infrarust.motd.text: "§aSurvival §7— §fWelcome!"
    networks:
      - minecraft
    restart: unless-stopped

  # ── Creative server ──────────────────────────────
  creative:
    image: itzg/minecraft-server
    container_name: creative
    environment:
      EULA: "TRUE"
      TYPE: "PAPER"
      GAMEMODE: "creative"
      MEMORY: "1G"
    labels:
      infrarust.enable: "true"
      infrarust.domains: "creative.mc.example.com"
      infrarust.proxy_mode: "client_only"
      infrarust.network: "main"
    networks:
      - minecraft
    restart: unless-stopped
```

Start everything with:

```bash
docker compose up -d
```

Players connecting to `mc.example.com` reach the lobby. `survival.mc.example.com` and `creative.mc.example.com` route to their respective servers. All of this happens without any per-server config files.

## How the labels work

Every Minecraft container needs `infrarust.enable: "true"` for Infrarust to pick it up. The rest is optional:

| Label | Default | What it does |
|-------|---------|--------------|
| `infrarust.enable` | required | Must be `"true"` for discovery. |
| `infrarust.domains` | `<container_name>.docker.local` | Comma-separated domains that route to this server. |
| `infrarust.port` | `25565` | Minecraft port inside the container. |
| `infrarust.proxy_mode` | `passthrough` | One of: `passthrough`, `client_only`, `offline`, `server_only`, `zero_copy`. |
| `infrarust.send_proxy_protocol` | `false` | Set to `"true"` or `"1"` to send PROXY protocol headers. |
| `infrarust.name` | none | Human-readable name for server switching. |
| `infrarust.network` | none | Network group for server switching. |
| `infrarust.motd.text` | none | Custom MOTD text shown in the server list. |

If you skip `infrarust.domains`, the container name becomes the domain with a `.docker.local` suffix. A container named `lobby` gets `lobby.docker.local`.

## Networking

All services share the `minecraft` bridge network. Backend servers don't publish any ports to the host. Players connect to `25565` on the host, Infrarust routes the traffic internally to the right container by its Docker network IP.

This is the recommended setup. Backend servers stay unreachable from outside Docker, and you avoid port conflicts between multiple servers.

::: tip
Set `network = "minecraft"` in your `infrarust.toml` `[docker]` section to match the Compose network name. This tells Infrarust which network IP to use when resolving container addresses.
:::

## Adding a server at runtime

Add a new service to your `docker-compose.yml` and run:

```bash
docker compose up -d newserver
```

Infrarust detects the new container within seconds through the Docker event stream. No restart needed.

Removing a server works the same way. Stop or remove the container and Infrarust drops it from the routing table automatically.

## Wildcard domains

You can route all subdomains of a domain to a single server:

```yaml
labels:
  infrarust.enable: "true"
  infrarust.domains: "*.mc.example.com"
```

Or list multiple domains on one server:

```yaml
labels:
  infrarust.enable: "true"
  infrarust.domains: "mc.example.com, play.example.com"
```

## Custom port

If your server runs on a non-standard port inside the container:

```yaml
modded:
  image: itzg/minecraft-server
  environment:
    EULA: "TRUE"
    SERVER_PORT: "25566"
  labels:
    infrarust.enable: "true"
    infrarust.domains: "modded.example.com"
    infrarust.port: "25566"
  networks:
    - minecraft
```

## Mixing Docker discovery and file-based servers

Docker and file providers feed into the same routing table. You can discover some servers from containers and define others as `.toml` files:

```toml
# infrarust.toml
bind = "0.0.0.0:25565"
servers_dir = "/app/config/servers"

[docker]
network = "minecraft"
```

```toml
# config/servers/external.toml
domains = ["external.example.com"]
addresses = ["192.168.1.50:25565"]
```

Containers get a provider ID like `docker@lobby`. File-based servers get `file@external.toml`. Both are visible in logs.

## Docker socket security

::: warning
Mounting `/var/run/docker.sock` gives Infrarust read access to all containers on the host. Always mount it read-only (`:ro`). On production systems, consider running Docker in rootless mode or using a socket proxy like [Tecnativa/docker-socket-proxy](https://github.com/Tecnativa/docker-socket-proxy) to restrict API access.
:::

## Troubleshooting

**Container not discovered**: Check that `infrarust.enable` is set to exactly `"true"` (string, not boolean). In YAML, quote it: `infrarust.enable: "true"`.

**Wrong IP resolved**: If Infrarust connects to the wrong container IP, set `network = "minecraft"` in your `[docker]` section to pin address resolution to the Compose network.

**Connection refused after startup**: Minecraft servers take time to start. Players connecting before the server is ready will get disconnected. The server container must be fully started before it can accept connections.

**Logs**: Set `RUST_LOG=debug` on the Infrarust container to see provider events:

```yaml
infrarust:
  environment:
    RUST_LOG: "debug"
```

## Next steps

- [Docker Provider reference](../configuration/providers/docker.md) for all provider options
- [Docker Deployment](./docker.md) for `docker run` commands and building the image
- [Proxy Modes](../configuration/proxy-modes/) to understand `passthrough` vs `client_only` and other modes
