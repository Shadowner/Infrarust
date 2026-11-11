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

All fields have defaults. Infrarust connects to the local Docker socket and starts scanning for labeled containers.

With an empty `[docker]` section, a container is only routed at `<container_name>.docker.local`, and a container whose `infrarust.domains` label claims any other domain is not routed at all. To let containers choose their own domains with that label, list those domains in [`allowed_domains`](#domain-allowlist).

### Provider options

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `endpoint` | string | `"unix:///var/run/docker.sock"` | Docker daemon to connect to. Use `tcp://host:2375` for remote daemons. The scheme (`unix://`, `tcp://` or `http://`) is required. Infrarust always connects to this value and does not read `DOCKER_HOST`. |
| `network` | string | none | Preferred Docker network for address resolution. |
| `poll_interval` | duration | `"30s"` | How often to rescan labelled containers (in addition to the event stream). A resync also runs as soon as the event stream connects, so containers that were already running are picked up even if Docker was unreachable during startup. |
| `reconnect_delay` | duration | `"5s"` | Initial delay before reconnecting after a daemon disconnection. |
| `allowed_domains` | string array | `[]` | Domains containers may claim with the `infrarust.domains` label. Entries accept `*` and `?`. See [Domain allowlist](#domain-allowlist). |
| `inherit_forwarding` | bool | `false` | Let containers inherit the proxy-wide `[forwarding]` mode and its secret. See [Forwarding inheritance](#forwarding-inheritance). |

Full example:

```toml
[docker]
endpoint = "unix:///var/run/docker.sock"
network = "minecraft"
poll_interval = "30s"
reconnect_delay = "5s"
allowed_domains = ["mc.example.com", "*.mc.example.com"]
inherit_forwarding = false
```

## Domain allowlist

A container can only claim a domain through `infrarust.domains` if that domain matches an entry of `allowed_domains`. The list is empty by default, so no label domain is accepted until you add it. Matching ignores case, and entries accept `*` (any run of characters) and `?` (one character):

```toml
[docker]
allowed_domains = ["play.example.com", "*.mc.example.com"]
```

The automatic domain `<container_name>.docker.local` is always allowed for the container that carries that name, with or without an allowlist.

If a label holds a wildcard domain such as `*.mc.example.com`, the allowlist must contain that same pattern as an entry. A broader entry does not admit a wildcard label.

A container that claims at least one domain outside the list is not routed at all, including for its other domains. Infrarust logs a warning with the line that would admit it, built from your current entries plus the refused domains:

```
WARN container not routed: its infrarust.domains label claims domains outside the allowlist. To allow them, set this line in [docker]: allowed_domains = ["play.example.com", "survival.example.com"]
```

The warning is logged once per container, not on every rescan. `[docker]` is part of `infrarust.toml`, so restart the proxy after you change the list.

## Forwarding inheritance

Containers do not inherit the proxy-wide [`[forwarding]`](../proxy-forwarding.md) mode. Each discovered server gets `forwarding_mode = "none"`, so the forwarding secret is never sent to a backend only because it carries `infrarust.*` labels.

If `[forwarding]` selects a mode other than `none` and a container uses an intercepted proxy mode (`client_only` or `offline`), Infrarust logs one warning: the container gets no player forwarding, and a backend that expects it rejects every player.

To give containers the proxy-wide mode and its secret, set:

```toml
[docker]
inherit_forwarding = true
```

Only enable this if you trust everyone who can start a labeled container on that Docker daemon.

With `inherit_forwarding = true` and a `[forwarding]` mode other than `none`, a container on `passthrough`, `zero_copy` or `server_only` is refused and logged as `server config rejected`, because those proxy modes cannot forward a verified player. Run every container on `client_only` or `offline` before you enable it.

## Container labels

Infrarust scans for containers with the label `infrarust.enable=true`. All other labels are optional.

| Label | Type | Default | Description |
|-------|------|---------|-------------|
| `infrarust.enable` | `"true"` | required | Must be `"true"` for the container to be discovered. |
| `infrarust.domains` | comma-separated | `<container_name>.docker.local` | Domains that route to this server. Each one must match [`allowed_domains`](#domain-allowlist). |
| `infrarust.port` | integer | `25565` | Minecraft port inside the container. |
| `infrarust.proxy_mode` | string | `passthrough` | One of: `passthrough`, `client_only`, `offline`, `server_only`, `zero_copy`. |
| `infrarust.send_proxy_protocol` | `"true"` or `"1"` | `false` | Send PROXY protocol headers to the backend. |
| `infrarust.name` | string | none | Human-readable name for server switching. |
| `infrarust.network` | string | none | Network group for server switching. |
| `infrarust.motd.text` | string | none | Custom MOTD text shown in the server list. |

If you omit `infrarust.domains`, the container name becomes the domain with a `.docker.local` suffix. A container named `mc-survival` gets the domain `mc-survival.docker.local`.

## Address resolution

Infrarust resolves the container address in this order:

1. The IP from the preferred Docker network (set via `network` in `[docker]` config). If `network` is not set or gives no IP, the IP of the first of the container's networks, by name in alphabetical order, that has one. The IP is combined with `infrarust.port` (default 25565) to form the backend address.
2. The host port the daemon published for `infrarust.port` (for example `25566:25565`). The backend address is then the published host IP and host port. A port published on `0.0.0.0` is reached through `127.0.0.1`, which only works when Infrarust runs on the Docker host itself.

Infrarust never uses the container name as a hostname. A container with no network IP and no published port for `infrarust.port` is not routed, and a warning is logged once:

```
WARN container not routed: it has no network IP and does not publish port 25565
```

This is the case for containers running with `network_mode: host`: Docker reports no IP and no published ports for them. Declare those servers in a [server file](./file.md) instead.

## docker-compose examples

Every domain used in an `infrarust.domains` label below must match an entry of `allowed_domains`, or the container is not routed.

### Single server

```yaml
services:
  infrarust:
    image: ghcr.io/shadowner/infrarust:latest
    ports:
      - "25565:25565"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - ./infrarust.toml:/app/config/infrarust.toml:ro
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

The matching `infrarust.toml`:

```toml
[docker]
allowed_domains = ["survival.example.com"]
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
      - ./infrarust.toml:/app/config/infrarust.toml:ro
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
      infrarust.motd.text: "§aSurvival §7| §fWelcome!"
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
allowed_domains = ["mc.example.com", "*.mc.example.com"]
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

```toml
[docker]
allowed_domains = ["modded.example.com"]
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

```toml
[docker]
allowed_domains = ["play.example.com"]
```

## How discovery works

On startup, Infrarust lists all running containers with `infrarust.enable=true` and registers them. It then subscribes to the Docker event stream and reacts to six events: `start`, `stop`, `die`, `destroy`, `pause`, and `unpause`.

When a container starts or unpauses, Infrarust inspects it, parses the labels, resolves the address, and adds it to the router. When a container stops, dies, pauses, or is destroyed, Infrarust removes it.

If the connection to the Docker daemon drops, Infrarust reconnects with exponential backoff (starting at `reconnect_delay`, capping at 60 seconds). After reconnecting, it re-scans all containers to catch any changes that happened while disconnected.

::: warning
The Docker feature must be compiled in. If you see the warning "docker configuration found but docker feature is not enabled, ignoring", rebuild Infrarust with the `docker` feature flag. The official Docker image includes it by default.
:::

## Mixing providers

Docker and file providers work side by side. You can define some servers as `.toml` files in `servers/` and discover others from Docker containers. Both feed into the same routing table.

::: tip
Docker-discovered servers get a provider ID in the format `docker@<container_name>`. File-based servers use `file@<filename>`. These IDs are visible in logs and the console.
:::
