# Docker Integration

Infrarust includes a robust Docker integration feature that automatically detects and proxies Minecraft servers running in Docker containers. This eliminates the need for manual proxy configuration when using containerized Minecraft servers.

## Overview

The Docker provider in Infrarust:
- Monitors Docker containers for real-time changes (start, stop, etc.)
- Automatically creates proxy configurations for Minecraft containers
- Supports custom domains, port mappings, and proxy modes
- Works with both bridge networks and port bindings

## Configuration

To enable Docker integration, add the following to your `config.yaml`:

```yaml
docker_provider:
  docker_host: "unix:///var/run/docker.sock"  # Docker daemon socket
  label_prefix: "infrarust"                   # Label prefix for container config
  polling_interval: 10                        # Fallback polling interval (seconds)
  watch: true                                 # Enable real-time container monitoring
  default_domains: ["docker.local"]           # Default domain suffix for containers
```

### Configuration Options

| Option | Description | Default |
|--------|-------------|---------|
| `docker_host` | Docker daemon socket/URL | `unix:///var/run/docker.sock` |
| `label_prefix` | Prefix for Docker labels | `infrarust` |
| `polling_interval` | Fallback polling interval | `10` |
| `watch` | Watch for container changes | `true` |
| `default_domains` | Default domain suffixes | `[]` |

### Connection Types

- **Unix Socket**: `unix:///var/run/docker.sock` (default on Linux)
- **TCP**: `tcp://localhost:2375` (remote Docker daemon)

## Container Configuration

Infrarust uses Docker labels to configure proxying for containers. Apply these labels to your Minecraft containers:

### Basic Labels

```yaml
labels:
  # Enable Infrarust proxy (required)
  infrarust.enable: true

  # Domain names (comma-separated)
  infrarust.domains: mc.example.com,mc-alt.example.com

  # Minecraft server port inside container
  infrarust.port: 25565
```

### Advanced Labels

```yaml
labels:
  # Proxy mode (passthrough, offline, client_only, server_only)
  infrarust.proxy_mode: passthrough

  # Enable PROXY protocol
  infrarust.proxy_protocol: true

  # Custom target address (override automatic detection)
  infrarust.address: custom-host:25565
```

## Container Discovery

Infrarust automatically determines the best address to use for connecting to containers:

1. First tries container IP addresses from Docker networks
2. Falls back to port bindings if no usable network IPs are found
3. Finally uses container name as hostname if nothing else works

## Example Docker Compose

Here's an example Docker Compose file with Infrarust proxy configuration:

```yaml
version: '3'
services:
  minecraft:
    image: itzg/minecraft-server
    ports:
      - "25565:25565"
    environment:
      EULA: "TRUE"
      MEMORY: "2G"
      TYPE: "PAPER"
      VERSION: "1.19.2"
    volumes:
      - minecraft_data:/data
    labels:
      infrarust.enable: "true"
      infrarust.domains: "mc.example.com,survival.example.com"
      infrarust.port: "25565"
      infrarust.proxy_mode: "passthrough"

  infrarust:
    image: infrarust/infrarust
    ports:
      - "25565:25565"
    volumes:
      - ./config:/config
      - /var/run/docker.sock:/var/run/docker.sock:ro
    depends_on:
      - minecraft

volumes:
  minecraft_data:
```

## Using Network Names

With a custom Docker network, your containers can reference each other by name:

```yaml
version: '3'
services:
  minecraft:
    # ... other config ...
    networks:
      - minecraft_network
    labels:
      infrarust.enable: "true"
      infrarust.domains: "mc.example.com"

  infrarust:
    # ... other config ...
    networks:
      - minecraft_network
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro

networks:
  minecraft_network:
    driver: bridge
```

## Multi-Server Setup

You can run multiple Minecraft servers with different domains:

```yaml
version: '3'
services:
  survival:
    image: itzg/minecraft-server
    # ... other config ...
    labels:
      infrarust.enable: "true"
      infrarust.domains: "survival.mc.example.com"

  creative:
    image: itzg/minecraft-server
    # ... other config ...
    labels:
      infrarust.enable: "true"
      infrarust.domains: "creative.mc.example.com"

  infrarust:
    # ... standard infrarust config ...
```

## Domain Auto-Generation

If no domains are specified via the `infrarust.domains` label, Infrarust automatically generates domain names:

1. If `default_domains` is empty: Uses `containername.docker.local`
2. If `default_domains` is set: Uses `containername.yourdomain.com` for each domain in the list

## Container Networks

If Infrarust and Minecraft containers are on different networks:

1. Add both to a shared network or
2. Expose the Minecraft ports and use port bindings

## Security Considerations

When mounting the Docker socket, you're giving Infrarust access to your Docker daemon. Consider:

1. Using read-only access: `/var/run/docker.sock:/var/run/docker.sock:ro`
2. Running Infrarust with minimal permissions
3. In production environments, consider using Docker's API with TLS authentication

## Performance Tuning

For large installations with many containers:

```yaml
docker_provider:
  polling_interval: 30  # Increase polling interval
```
