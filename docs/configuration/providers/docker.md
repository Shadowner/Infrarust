---
title: Docker Provider
description: Auto-discover Minecraft servers from Docker containers using labels, with real-time updates when containers start or stop.
---

# Docker Provider

The Docker provider discovers Minecraft servers from running containers. Add `infrarust.*` labels to your containers and Infrarust registers them automatically. When a container starts or stops, the proxy updates its routing table in real time.

## Enabling the provider

Add a `[docker]` section to your `infrarust.toml`:

```toml
[docker]
```

That's it. All fields have defaults. Infrarust connects to the local Docker socket and starts scanning for labeled containers.

### Provider options

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `endpoint` | string | `"unix:///var/run/docker.sock"` | Docker daemon socket. Use `tcp://host:2375` for remote daemons. |
| `network` | string | none | Preferred Docker network for address resolution. |
| `poll_interval` | duration | `"30s"` | Fallback polling interval. |
| `reconnect_delay` | duration | `"5s"` | Initial delay before reconnecting after a daemon disconnection. |

Full example:

```toml
[docker]
endpoint = "unix:///var/run/docker.sock"
network = "minecraft"
poll_interval = "30s"
reconnect_delay = "5s"
```

## Container labels

Infrarust scans for containers with the label `infrarust.enable=true`. All other labels are optional.

| Label | Type | Default | Description |
|-------|------|---------|-------------|
| `infrarust.enable` | `"true"` | required | Must be `"true"` for the container to be discovered. |
| `infrarust.domains` | comma-separated | `<container_name>.docker.local` | Domains that route to this server. |
| `infrarust.port` | integer | `25565` | Minecraft port inside the container. |
| `infrarust.proxy_mode` | string | `passthrough` | One of: `passthrough`, `client_only`, `offline`, `server_only`, `zero_copy`. |
| `infrarust.send_proxy_protocol` | `"true"` or `"1"` | `false` | Send PROXY protocol headers to the backend. |
| `infrarust.name` | string | none | Human-readable name for server switching. |
| `infrarust.network` | string | none | Network group for server switching. |
| `infrarust.motd.text` | string | none | Custom MOTD text shown in the server list. |

If you omit `infrarust.domains`, the container name becomes the domain with a `.docker.local` suffix. A container named `mc-survival` gets the domain `mc-survival.docker.local`.

## Address resolution

Infrarust resolves the container address in this order:

1. The IP from the preferred Docker network (set via `network` in `[docker]` config or `infrarust.network` label), or the first available network IP.
2. Port bindings from the host config (e.g., `0.0.0.0:25565` mapped port).
3. The container name as a hostname (last resort).

The resolved IP is combined with `infrarust.port` (default 25565) to form the backend address.

## docker-compose examples

### Single server

```yaml
services:
  infrarust:
    image: ghcr.io/shadowner/infrarust:latest
    ports:
      - "25565:25565"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - ./infrarust.toml:/app/infrarust.toml:ro
    restart: unless-stopped

  mc-survival:
    image: itzg/minecraft-server
    environment:
      EULA: "TRUE"
      TYPE: "PAPER"
    labels:
      infrarust.enable: "true"
      infrarust.domains: "survival.example.com"
    restart: unless-stopped
```

With this setup, players connecting to `survival.example.com` are routed to the `mc-survival` container on port 25565. No static server config files needed.

### Multiple servers on a shared network

```yaml
networks:
  minecraft:
    driver: bridge

services:
  infrarust:
    image: ghcr.io/shadowner/infrarust:latest
    ports:
      - "25565:25565"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - ./infrarust.toml:/app/infrarust.toml:ro
    networks:
      - minecraft
    restart: unless-stopped

  lobby:
    image: itzg/minecraft-server
    environment:
      EULA: "TRUE"
    labels:
      infrarust.enable: "true"
      infrarust.domains: "mc.example.com"
      infrarust.proxy_mode: "client_only"
      infrarust.network: "main"
    networks:
      - minecraft
    restart: unless-stopped

  survival:
    image: itzg/minecraft-server
    environment:
      EULA: "TRUE"
      TYPE: "PAPER"
      DIFFICULTY: "hard"
    labels:
      infrarust.enable: "true"
      infrarust.domains: "survival.mc.example.com"
      infrarust.proxy_mode: "client_only"
      infrarust.network: "main"
      infrarust.motd.text: "§aSurvival §7— §fWelcome!"
    networks:
      - minecraft
    restart: unless-stopped

  creative:
    image: itzg/minecraft-server
    environment:
      EULA: "TRUE"
      TYPE: "PAPER"
      GAMEMODE: "creative"
    labels:
      infrarust.enable: "true"
      infrarust.domains: "creative.mc.example.com"
      infrarust.proxy_mode: "client_only"
      infrarust.network: "main"
      infrarust.port: "25565"
    networks:
      - minecraft
    restart: unless-stopped
```

The matching `infrarust.toml`:

```toml
[docker]
network = "minecraft"
```

Setting `network = "minecraft"` tells Infrarust to prefer IPs from the `minecraft` Docker network when resolving container addresses. This avoids issues with containers that are attached to multiple networks.

### Custom port

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
```

### PROXY protocol

If your backend expects PROXY protocol headers (for IP forwarding):

```yaml
  backend:
    image: itzg/minecraft-server
    labels:
      infrarust.enable: "true"
      infrarust.domains: "play.example.com"
      infrarust.send_proxy_protocol: "true"
```

## How discovery works

On startup, Infrarust lists all running containers with `infrarust.enable=true` and registers them. It then subscribes to the Docker event stream and reacts to six events: `start`, `stop`, `die`, `destroy`, `pause`, and `unpause`.

When a container starts or unpauses, Infrarust inspects it, parses the labels, resolves the address, and adds it to the router. When a container stops, dies, pauses, or is destroyed, Infrarust removes it.

If the connection to the Docker daemon drops, Infrarust reconnects with exponential backoff (starting at `reconnect_delay`, capping at 60 seconds). After reconnecting, it re-scans all containers to catch any changes that happened while disconnected.

::: warning
The Docker feature must be compiled in. If you see a warning like "docker config found but docker feature is not enabled", rebuild Infrarust with the `docker` feature flag. The official Docker image includes it by default.
:::

## Mixing providers

Docker and file providers work side by side. You can define some servers as `.toml` files in `servers/` and discover others from Docker containers. Both feed into the same routing table.

::: tip
Docker-discovered servers get a provider ID in the format `docker@<container_name>`. File-based servers use `file@<filename>`. These IDs are visible in logs and the console.
:::
