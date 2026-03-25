# Configuration Reference

This document details all available configuration options in Infrarust.

## Configuration Structure

Infrarust uses two types of configuration files:

```
infrarust/
├── config.yaml         # Global configuration
└── proxies/           # Server configurations
    ├── hub.yml
    ├── survival.yml
    └── creative.yml
```

## Main Configuration (config.yaml)

The main configuration file supports the following options:

```yaml
# Basic Configuration
bind: "0.0.0.0:25565"           # Address to bind the proxy to
keepAliveTimeout: 30s           # Connection keepalive timeout

# File Provider Configuration
file_provider:
  proxies_path: ["./proxies"]   # Path to proxy configurations
  file_type: "yaml"             # File type (currently only yaml supported)
  watch: true                   # Enable hot-reload of configurations

# Docker Provider Configuration
docker_provider:
  docker_host: "unix:///var/run/docker.sock"  # Docker daemon socket
  label_prefix: "infrarust"                   # Label prefix for containers
  polling_interval: 10                        # Polling interval in seconds
  watch: true                                 # Watch for container changes
  default_domains: []                         # Default domains for containers

# Server Managers Configuration
managers_config:
  pterodactyl:
    enabled: true
    api_key: "your_api_key"
    base_url: "https://pterodactyl.example.com"
  crafty:
    enabled: true
    api_key: "your_api_key"
    base_url: "https://crafty.example.com"

# Proxy Protocol Configuration (Receive)
proxy_protocol:
  enabled: true                    # Enable proxy protocol support
  receive_enabled: true            # Accept incoming proxy protocol
  receive_timeout_secs: 5          # Timeout for receiving proxy protocol header
  receive_allowed_versions: [1, 2] # Allowed proxy protocol versions

# Cache Configuration
cache:
  status_ttl_seconds: 30        # TTL for status cache entries
  max_status_entries: 1000      # Maximum number of status cache entries

# Telemetry Configuration
telemetry:
  enabled: false               # Enable telemetry collection
  export_interval_seconds: 30  # Export interval
  export_url: "http://..."     # Export destination (optional)
  enable_metrics: false        # Enable metrics collection
  enable_tracing: false        # Enable distributed tracing

# Logging Configuration
logging:
  debug: true                  # Enable debug mode
  use_color: true              # Use colors in console output
  use_icons: true              # Use icons in console output
  show_timestamp: true         # Show timestamps in logs
  time_format: "%Y-%m-%d %H:%M:%S"  # Timestamp format
  show_target: true            # Show log target
  show_fields: true            # Show log fields
  template: "{timestamp} [{level}] {message}"  # Log template
  regex_filter: "^(pattern)"   # Filter logs by regex pattern
  min_level: "info"            # Global minimum log level
  log_types:                   # Per-component log levels
    supervisor: "info"
    server_manager: "info"
    packet_processing: "debug"
    proxy_protocol: "debug"
    ban_system: "info"
    authentication: "info"
    filter: "info"
    config_provider: "info"
    cache: "debug"
    motd: "warn"
    telemetry: "error"
  exclude_types:               # Exclude noisy log types
    - "tcp_connection"
    - "packet_processing"
    - "cache"

# Filter Configuration
filters:
  rate_limiter:
    enabled: true
    requests_per_minute: 600   # Maximum requests per minute
    burst_size: 10             # Burst size for rate limiting

# Default MOTD Configuration
motds:
  unreachable:
    version_name: "Infrarust Unreachable"
    protocol_version: 760
    max_players: 100
    online_players: 0
    text: "Server Unreachable"
    favicon: ""
```

## Server Configuration (proxies/*.yml)

Each server configuration file in the proxies directory can contain:

```yaml
domains:
  - "play.example.com"      # Domain names for this server
addresses:
  - "localhost:25566"       # Target server addresses

sendProxyProtocol: false    # Send PROXY protocol to backend
proxy_protocol_version: 2   # PROXY protocol version to use (1 or 2)

proxyMode: "passthrough"    # Proxy mode (passthrough/client_only/offline/server_only)

# Server Manager Configuration (optional)
server_manager:
  provider_name: Local       # Local | Pterodactyl | Crafty | Docker
  server_id: "my_server"
  empty_shutdown_time: 300   # Shutdown after idle time (seconds)
  local_provider:            # Only for Local provider
    executable: "java"
    working_dir: "/path/to/server"
    args:
      - "-jar"
      - "server.jar"
    startup_string: 'For help, type "help"'

# MOTD Configuration (per-server)
motds:
  online:
    enabled: true
    text: "Welcome to our server!"
    version_name: "Paper 1.20.4"
    max_players: 100
    online_players: 42
    protocol_version: 765
    favicon: "./icons/server.png"
    samples:
      - name: "Steve"
        id: "069a79f4-44e9-4726-a5be-fca90e38aaf5"
  offline:
    enabled: true
    text: "Server Sleeping - Connect to wake it up!"
  # Other states: starting, stopping, shutting_down, crashed, unreachable, unable_status

# Per-server Filter Configuration
filters:
  rate_limiter:
    enabled: true
    requests_per_minute: 600
    burst_size: 10
  ip_filter:
    enabled: true
    whitelist: ["127.0.0.1"]
    blacklist: []
  id_filter:
    enabled: true
    whitelist: ["uuid1", "uuid2"]
    blacklist: []
  name_filter:
    enabled: true
    whitelist: ["player1"]
    blacklist: []
  ban:
    enabled: true
    storage_type: "file"
    file_path: "bans.json"

# Per-server Cache Configuration
caches:
  status_ttl_seconds: 30
  max_status_entries: 1000
```

For complete server configuration examples, see the [config_examples on GitHub](https://github.com/Shadowner/Infrarust/tree/main/config_examples/proxies).

## Feature Reference

### Proxy Modes

| Mode | Description |
|------|-------------|
| `passthrough` | Direct proxy, compatible with all Minecraft versions |
| `client_only` | For premium clients connecting to offline servers |
| `server_only` | For scenarios where server authentication needs handling |
| `offline` | For offline clients and servers |

### Server Managers

Infrarust can automatically start and stop Minecraft servers based on player activity.

#### Pterodactyl Integration

```yaml
managers_config:
  pterodactyl:
    enabled: true
    api_key: "your_api_key"
    base_url: "https://pterodactyl.example.com"
```

Then in your server config:

```yaml
server_manager:
  provider_name: Pterodactyl
  server_id: "your_server_uuid"
  empty_shutdown_time: 300
```

#### Crafty Controller Integration

```yaml
managers_config:
  crafty:
    enabled: true
    api_key: "your_api_key"
    base_url: "https://crafty.example.com"
```

Then in your server config:

```yaml
server_manager:
  provider_name: Crafty
  server_id: "your_server_uuid"
```

#### Local Server Management

For locally managed servers:

```yaml
server_manager:
  provider_name: Local
  server_id: "local_server"
  empty_shutdown_time: 300
  local_provider:
    executable: "java"
    working_dir: "/path/to/server"
    args:
      - "-jar"
      - "server.jar"
    startup_string: 'For help, type "help"'
```

### Docker Integration

Infrarust can automatically proxy Minecraft containers:

```yaml
docker_provider:
  docker_host: "unix:///var/run/docker.sock"
  label_prefix: "infrarust"
  polling_interval: 10
  watch: true
  default_domains: ["docker.local"]
```

Container configuration is done through Docker labels:
- `infrarust.enable=true` - Enable proxying for the container
- `infrarust.domains=mc.example.com,mc2.example.com` - Domains for the container
- `infrarust.port=25565` - Minecraft port inside the container
- `infrarust.proxy_mode=passthrough` - Proxy mode
- `infrarust.proxy_protocol=true` - Enable PROXY protocol

### Proxy Protocol

Configure PROXY protocol for receiving client information from load balancers:

```yaml
proxy_protocol:
  enabled: true
  receive_enabled: true
  receive_timeout_secs: 5
  receive_allowed_versions: [1, 2]
```

To send PROXY protocol to backend servers, configure in the server config:

```yaml
sendProxyProtocol: true
proxy_protocol_version: 2
```

### Telemetry

Telemetry configuration allows monitoring of the proxy via OpenTelemetry:

```yaml
telemetry:
  enabled: true
  export_interval_seconds: 10
  export_url: "http://localhost:4317"
  enable_metrics: true
  enable_tracing: true
```

### MOTD Configuration

Configure server list display for different server states:

| State | Description |
|-------|-------------|
| `online` | Server is running and reachable |
| `offline` | Server is sleeping/stopped |
| `starting` | Server is starting up |
| `stopping` | Server is gracefully stopping |
| `shutting_down` | Countdown to shutdown (supports `${seconds_remaining}` placeholder) |
| `crashed` | Server crashed |
| `unreachable` | Cannot reach server |
| `unable_status` | Cannot get server status |

MOTD fields:
- `enabled` - Enable this MOTD state
- `text` - Server description (supports Minecraft color codes)
- `version_name` - Version text to display
- `protocol_version` - Minecraft protocol version number
- `max_players` - Maximum player count
- `online_players` - Current player count
- `favicon` - Server icon (base64 encoded PNG or file path)
- `samples` - Player list samples (array of `{name, id}`)

For complete MOTD examples, see [local-server.yaml](https://github.com/Shadowner/Infrarust/blob/main/config_examples/proxies/local-server.yaml).

### Cache Configuration

Configure status caching:

```yaml
cache:
  status_ttl_seconds: 30    # Time-to-live for status cache entries
  max_status_entries: 1000  # Maximum number of status cache entries
```

### Filter Configuration

#### Rate Limiter

Controls the number of connections from a single source:

```yaml
rate_limiter:
  enabled: true
  requests_per_minute: 600  # Maximum requests per minute
  burst_size: 10            # Burst size allowance
```

#### Access Lists

Available for IP addresses, UUIDs, and player names:

```yaml
ip_filter:  # or id_filter / name_filter
  enabled: true
  whitelist: ["value1", "value2"]
  blacklist: ["value3"]
```

#### Ban System

Configure persistent player bans:

```yaml
ban:
  enabled: true
  storage_type: "file"  # file, redis, or database
  file_path: "bans.json"
  enable_audit_log: true
  audit_log_path: "bans_audit.log"
  audit_log_rotation:
    max_size: 10485760  # 10MB
    max_files: 5
    compress: true
  auto_cleanup_interval: 3600  # 1 hour
  cache_size: 10000
```

### Logging Configuration

Fine-tune log output:

```yaml
logging:
  debug: true
  use_color: true
  use_icons: true
  show_timestamp: true
  time_format: "%Y-%m-%d %H:%M:%S"
  show_target: true
  show_fields: true
  template: "{timestamp} [{level}] {message}"
  regex_filter: "^(pattern)"
  min_level: "info"
  log_types:
    supervisor: "info"
    server_manager: "info"
    packet_processing: "debug"
    ban_system: "info"
    authentication: "info"
    telemetry: "warn"
  exclude_types:
    - "tcp_connection"
    - "cache"
```

Available log types: `tcp_connection`, `supervisor`, `server_manager`, `packet_processing`, `ban_system`, `authentication`, `telemetry`, `config_provider`, `proxy_protocol`, `cache`, `filter`, `proxy_mode`, `motd`

## Advanced Features

### Hot Reload

When `file_provider.watch` is enabled, configuration changes are automatically detected and applied without restart.

> Active by default

### Docker Integration

When `docker_provider.watch` is enabled, container changes are automatically detected and proxies are updated accordingly.

### Ban System

The ban system provides persistent bans with flexible storage options and audit logging.

## Need Help?

- Report issues on [GitHub](https://github.com/shadowner/infrarust/issues)
- Join our [Discord](https://discord.gg/sqbJhZVSgG)
- Check the [documentation](https://infrarust.dev)
