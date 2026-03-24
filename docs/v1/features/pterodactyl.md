# Pterodactyl Integration

Infrarust includes integration with Pterodactyl, a popular open-source game server management panel. This integration allows Infrarust to monitor server status and automatically start servers when players connect.

## Overview

The Pterodactyl provider in Infrarust:
- Monitors server status in real-time (Starting, Running, Stopping, Stopped, Crashed)
- Automatically starts servers when players attempt to connect
- Supports remote server control (start, stop, restart)
- Automatically shuts down empty servers after a configurable timeout
- Uses Pterodactyl's Client API for communication

## Configuration

### Manager Configuration

To enable Pterodactyl integration, add the following to your `config.yaml`:

```yaml
managers_config:
  pterodactyl:
    enabled: true
    api_key: "your_pterodactyl_api_key" # Must be a client ApiKLey that start with "ptlc_"
    base_url: "https://panel.example.com"
```

### Configuration Options

| Option | Description | Required |
|--------|-------------|----------|
| `enabled` | Enable Pterodactyl integration | Yes |
| `api_key` | Client API key from Pterodactyl panel | Yes |
| `base_url` | Base URL of your Pterodactyl panel | Yes |

## Server Configuration

To configure a proxy to use Pterodactyl for server management, add the `server_manager` section to your proxy configuration file:

```yaml
domains:
  - "mc.example.com"
addresses:
  - "192.168.1.100:25565"
proxyMode: "passthrough"

server_manager:
  provider_name: Pterodactyl
  server_id: "de0d8f2d"
  empty_shutdown_time: 30

motds:
  online:
    text: "Server Online"
  offline:
    text: "Server Offline - Connecting will start the server"
```

### Server Configuration Options

| Option | Description | Required |
|--------|-------------|----------|
| `provider_name` | Must be `Pterodactyl` for Pterodactyl panel | Yes |
| `server_id` | The server identifier from Pterodactyl panel | Yes |
| `empty_shutdown_time` | Seconds to wait before shutting down an empty server | No |

## Complete Example

### Main Configuration (`config.yaml`)

```yaml
bind: "0.0.0.0:25565"

file_provider:
  proxies_path:
    - "./proxies"
  watch: true

managers_config:
  pterodactyl:
    enabled: true
    api_key: "ptlc_xxxxxxxxxxxxxxxxxxxx"
    base_url: "https://panel.example.com"
```

### Proxy Configuration (`proxies/survival.yaml`)

```yaml
domains:
  - "survival.example.com"
  - "play.example.com"
addresses:
  - "192.168.1.100:25565"
sendProxyProtocol: false
proxyMode: "passthrough"

server_manager:
  provider_name: Pterodactyl
  server_id: "de0d8f2d"
  empty_shutdown_time: 300

motds:
  online:
    version_name: "Survival Server"
    text: "Welcome to Survival!"
  offline:
    version_name: "Server Starting..."
    text: "Server is offline. Join to start it!"
```

## Server Status States

The Pterodactyl integration recognizes the following server states:

| State | Description |
|-------|-------------|
| `Starting` | Server is starting up |
| `Running` | Server is online and accepting connections |
| `Stopping` | Server is shutting down |
| `Stopped` | Server is offline |
| `Crashed` | Server has crashed and may need attention |

## Auto-Shutdown Feature

When `empty_shutdown_time` is configured, Infrarust will automatically stop the server after the specified number of seconds when no players are connected. This helps save resources when servers are not in use.

```yaml
server_manager:
  provider_name: Pterodactyl
  server_id: "de0d8f2d"
  empty_shutdown_time: 300  # Shutdown after 5 minutes of no players
```

## API Capabilities

The integration supports the following operations through Pterodactyl's Client API:

- **Get Server Status**: Check current server state and resource usage
- **Start Server**: Start an offline server
- **Stop Server**: Gracefully stop a running server
- **Restart Server**: Restart a running server

### API Endpoints Used

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/client/servers/{id}` | GET | Get server information |
| `/api/client/servers/{id}/resources` | GET | Get server status and resources |
| `/api/client/servers/{id}/power` | POST | Control server power state |
