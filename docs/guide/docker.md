---
title: Docker Deployment
description: Run Infrarust in Docker with volume mounts, environment variables, Docker Compose, and automatic server discovery from container labels.
---

# Docker Deployment

Infrarust ships a minimal Docker image built from `scratch`. The image contains a statically linked binary, CA certificates, and nothing else. It supports x86_64, aarch64, and armv7.

## Directory structure

Infrarust inside Docker expects a config volume mounted at `/app/config`. The layout:

```
config/
├── infrarust.toml
└── servers/
    └── survival.toml
```

Set `servers_dir` in your `infrarust.toml` to the path inside the container:

```toml
bind = "0.0.0.0:25565"
servers_dir = "/app/config/servers"
```

## Running with docker run

```bash
docker run -d \
  --name infrarust \
  -p 25565:25565 \
  -v ./config:/app/config \
  ghcr.io/shadowner/infrarust:latest \
  --config /app/config/infrarust.toml
```

The `--config` flag tells Infrarust where to find `infrarust.toml` inside the container. If you don't pass it, the binary looks for `infrarust.toml` in the working directory (`/app`).

## Docker Compose

A minimal `docker-compose.yml`:

```yaml
services:
  infrarust:
    image: ghcr.io/shadowner/infrarust:latest
    command: ["--config", "/app/config/infrarust.toml"]
    ports:
      - "25565:25565"
    volumes:
      - ./config:/app/config
    restart: unless-stopped
```

## Environment variables

Infrarust reads one environment variable at runtime:

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Log level filter. Overrides the `--log-level` CLI flag. Accepts `trace`, `debug`, `info`, `warn`, `error`, or module-level filters like `infrarust_core=debug`. |

Set it in your Compose file or `docker run`:

```bash
docker run -d \
  --name infrarust \
  -p 25565:25565 \
  -e RUST_LOG=debug \
  -v ./config:/app/config \
  ghcr.io/shadowner/infrarust:latest \
  --config /app/config/infrarust.toml
```

Or in Compose:

```yaml
services:
  infrarust:
    image: ghcr.io/shadowner/infrarust:latest
    command: ["--config", "/app/config/infrarust.toml"]
    ports:
      - "25565:25565"
    volumes:
      - ./config:/app/config
    environment:
      RUST_LOG: "info"
    restart: unless-stopped
```

## CLI flags

These flags are passed after the image name in `docker run`, or in the `command` field of Compose:

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config <PATH>` | `infrarust.toml` | Path to the proxy configuration file |
| `-b, --bind <ADDR>` | from config | Override the bind address |
| `-l, --log-level <LEVEL>` | `info` | Log level filter (overridden by `RUST_LOG`) |

## Auto-discovery with Docker labels

When Infrarust runs alongside your Minecraft servers in Docker, it can discover them automatically through container labels. No static server config files needed.

Add a `[docker]` section to your `infrarust.toml`:

```toml
servers_dir = "/app/config/servers"

[docker]
```

Then label your Minecraft containers:

```yaml
services:
  infrarust:
    image: ghcr.io/shadowner/infrarust:latest
    command: ["--config", "/app/config/infrarust.toml"]
    ports:
      - "25565:25565"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - ./config:/app/config
    restart: unless-stopped

  survival:
    image: itzg/minecraft-server
    environment:
      EULA: "TRUE"
      TYPE: "PAPER"
    labels:
      infrarust.enable: "true"
      infrarust.domains: "survival.example.com"
    restart: unless-stopped
```

Infrarust scans for containers with `infrarust.enable=true`, reads their labels, resolves their network address, and registers them as backend servers. When containers start or stop, the routing table updates in real time.

::: warning
Mounting the Docker socket (`/var/run/docker.sock`) gives Infrarust read access to all containers on the host. Mount it read-only (`:ro`) and understand the security implications before deploying this in production.
:::

The full set of container labels and provider options is documented in [Docker Provider](../configuration/providers/docker.md).

## Networking

When Infrarust and your Minecraft servers share a Docker network, containers communicate by internal IP. This is the recommended setup because it avoids port publishing on the host for backend servers.

```yaml
networks:
  minecraft:
    driver: bridge

services:
  infrarust:
    image: ghcr.io/shadowner/infrarust:latest
    command: ["--config", "/app/config/infrarust.toml"]
    ports:
      - "25565:25565"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - ./config:/app/config
    networks:
      - minecraft
    restart: unless-stopped

  survival:
    image: itzg/minecraft-server
    environment:
      EULA: "TRUE"
    labels:
      infrarust.enable: "true"
      infrarust.domains: "survival.example.com"
    networks:
      - minecraft
    restart: unless-stopped
```

Tell Infrarust which Docker network to prefer for address resolution:

```toml
[docker]
network = "minecraft"
```

Without this setting, Infrarust picks the first available network IP from each container. If your containers are on multiple networks, set `network` to avoid resolving to the wrong IP.

## Building the image yourself

The included `Dockerfile` uses a multi-stage build: Alpine + Rust for compilation, `scratch` for the final image.

```bash
docker build -t infrarust .
```

The build detects the host architecture and targets it automatically. Cross-compilation is handled through Docker buildx:

```bash
docker buildx build --platform linux/amd64,linux/arm64 -t infrarust .
```

## Connecting to external servers

If your Minecraft servers run outside Docker (bare metal, VMs, another host), you don't need the Docker provider or socket mount. Use static server config files instead:

```yaml
services:
  infrarust:
    image: ghcr.io/shadowner/infrarust:latest
    command: ["--config", "/app/config/infrarust.toml"]
    ports:
      - "25565:25565"
    volumes:
      - ./config:/app/config
    restart: unless-stopped
```

Define your servers as `.toml` files in `config/servers/`:

```toml
# config/servers/survival.toml
domains = ["survival.example.com"]
addresses = ["192.168.1.50:25565"]
```

Infrarust watches the `servers/` directory and picks up new or changed files without a restart.
