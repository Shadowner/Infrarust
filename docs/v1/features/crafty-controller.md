# Crafty Controller Integration

Infrarust includes integration with Crafty Controller, a popular web-based Minecraft server management panel. This integration allows Infrarust to monitor server status and automatically start servers when players connect.

## Overview

The Crafty Controller provider in Infrarust:
- Monitors server status in real-time (Running, Stopped, Crashed)
- Automatically starts servers when players attempt to connect
- Supports remote server control (start, stop, restart)
- Uses Crafty Controller's REST API for communication

## Configuration

### Manager Configuration

To enable Crafty Controller integration, add the following to your `config.yaml`:

```yaml
managers_config:
  crafty:
    enabled: true
    api_key: "your_crafty_api_key"
    base_url: "https://crafty.example.com"
```

### Configuration Options

| Option | Description | Required |
|--------|-------------|----------|
| `enabled` | Enable Crafty Controller integration | Yes |
| `api_key` | API key for Bearer token authentication | Yes |
| `base_url` | Base URL of your Crafty Controller instance | Yes |

## Server Configuration

To configure a proxy to use Crafty Controller for server management, add the `server_manager` section to your proxy configuration file:

```yaml
domains:
  - "mc.example.com"
addresses:
  - "127.0.0.1:25565"
proxyMode: "passthrough"

server_manager:
  provider_name: Crafty
  server_id: "your-server-uuid"
  empty_shutdown_time: 300  # Shutdown after 5 minutes of no players

motds:
  online:
    text: "Server Online"
  offline:
    text: "Server Offline - Connecting will start the server"
```

### Server Configuration Options

| Option | Description | Required |
|--------|-------------|----------|
| `provider_name` | Must be `Crafty` for Crafty Controller | Yes |
| `server_id` | The UUID of the server in Crafty Controller | Yes |

## Complete Example

### Main Configuration (`config.yaml`)

```yaml
bind: "0.0.0.0:25565"

file_provider:
  proxies_path:
    - "./proxies"
  watch: true

managers_config:
  crafty:
    enabled: true
    api_key: "your_crafty_api_key"
    base_url: "https://crafty.example.com"
```

### Proxy Configuration (`proxies/survival.yaml`)

```yaml
domains:
  - "survival.example.com"
addresses:
  - "127.0.0.1:25565"
proxyMode: "passthrough"

server_manager:
  provider_name: Crafty
  server_id: "550e8400-e29b-41d4-a716-446655440000"
  empty_shutdown_time: 300  # Shutdown after 5 minutes of no players

motds:
  online:
    version_name: "Survival Server"
    text: "Welcome to Survival!"
  offline:
    version_name: "Server Starting..."
    text: "Server is offline. Join to start it!"
```

## Server Status States

The Crafty Controller integration recognizes the following server states:

| State | Description |
|-------|-------------|
| `Running` | Server is online and accepting connections |
| `Stopped` | Server is offline |
| `Crashed` | Server has crashed and needs attention |

## Auto-Shutdown Feature

When `empty_shutdown_time` is configured, Infrarust will automatically stop the server after the specified number of seconds when no players are connected. This helps save resources when servers are not in use.

```yaml
server_manager:
  provider_name: Pterodactyl
  server_id: "de0d8f2d"
  empty_shutdown_time: 300  # Shutdown after 5 minutes of no players
```

## API Capabilities

The integration supports the following operations through Crafty Controller's API:

- **Get Server Status**: Check if the server is running, stopped, or crashed
- **Start Server**: Start an offline server
- **Stop Server**: Gracefully stop a running server
- **Restart Server**: Restart a running server
