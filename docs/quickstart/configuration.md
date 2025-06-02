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
domains: ["example.com"]        # Default domains (optional)
addresses: ["localhost:25566"]  # Default target addresses (optional)

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
  use_color: true              # Use colors in console output
  use_icons: true              # Use icons in console output
  show_timestamp: true         # Show timestamps in logs
  time_format: "%Y-%m-%d %H:%M:%S%.3f"  # Timestamp format
  show_target: false           # Show log target
  show_fields: false           # Show log fields
  template: "{timestamp} {level}: {message}"  # Log template
  field_prefixes: {}           # Field prefix mappings

# Default MOTD Configuration
motds:
  unknown:                    # MOTD for unknown servers
    version: "1.20.1"        # Minecraft version to display
    max_players: 100         # Maximum players to show
    online_players: 0        # Online players to show
    text: "Unknown server" # Server description
    favicon: "data:image/png;base64,..." # Server icon (optional)
  unreachable:              # MOTD for unreachable servers
    # Same options as 'unknown'
```

## Server Configuration (proxies/*.yml)

Each server configuration file in the proxies directory can contain:

```yaml
domains:
  - "play.example.com"      # Domain names for this server
addresses:
  - "localhost:25566"       # Target server addresses

sendProxyProtocol: false    # Enable PROXY protocol support
proxy_protocol_version: 2   # PROXY protocol version to use (1 or 2)

proxyMode: "passthrough"    # Proxy mode (passthrough/client_only/offline/server_only)


# MOTD Configuration (overrides default / server motd)
motd:
  version: "1.20.1"       # Can be set to any text
  max_players: 100
  online_players: 0
  text: "Welcome to my server!"
  favicon: "data:image/png;base64,..."

### DOWN BELOW IMPLEMENTED BUT NOT YET SUPPORTED ###

# Cache Configuration
caches:
  status_ttl_seconds: 30    # TTL for status cache entries
  max_status_entries: 1000  # Maximum number of status cache entries

# Filter Configuration
filters:
  rate_limiter:
    requestLimit: 10        # Maximum requests per window
    windowLength: 1s        # Time window for rate limiting
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
    storage_type: "file"    # Storage type (file/redis/database)
    file_path: "bans.json"  # Path to ban storage file
    enable_audit_log: true  # Enable ban audit logging
    audit_log_path: "bans_audit.log"  # Path to audit log
    audit_log_rotation:     # Log rotation settings
      max_size: 10485760    # Max log size (10MB)
      max_files: 5          # Max number of log files
      compress: true        # Compress rotated logs
    auto_cleanup_interval: 3600  # Auto cleanup interval in seconds
    cache_size: 10000      # Ban cache size
```

## Feature Reference

### Proxy Modes

| Mode | Description |
|------|-------------|
| `passthrough` | Direct proxy, compatible with all Minecraft versions |
| `client_only` | For premium clients connecting to offline servers |
| `server_only` | For scenarios where server authentication needs handling |
| `offline` | For offline clients and servers |

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

### Telemetry

Telemetry configuration allows monitoring of the proxy:

```yaml
telemetry:
  enabled: false
  export_interval_seconds: 30
  export_url: "http://..."
  enable_metrics: false
  enable_tracing: false
```

### MOTD Configuration

Configure server list display:

```yaml
motd:
  version: "1.20.1"        # Protocol version to display
  max_players: 100         # Maximum player count
  online_players: 0        # Current player count
  text: "Text"      # Server description
  favicon: "base64..."     # Server icon (base64 encoded PNG)
```

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
  requestLimit: 10    # Maximum requests
  windowLength: 1s    # Time window
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
  use_color: true
  use_icons: true
  show_timestamp: true
  time_format: "%Y-%m-%d %H:%M:%S%.3f"
  show_target: false
  show_fields: false
  template: "{timestamp} {level}: {message}"
  field_prefixes: {}
```

## Advanced Features

### Hot Reload

When `file_provider.watch` is enabled, configuration changes are automatically detected and applied without restart.

> Active by default

### Docker Integration

When `docker_provider.watch` is enabled, container changes are automatically detected and proxies are updated accordingly.

### Ban System

The ban system provides persistent bans with flexible storage options and audit logging.

## Need Help?

- 🐛 Report issues on [GitHub](https://github.com/shadowner/infrarust/issues)
- 💬 Join our [Discord](https://discord.gg/sqbJhZVSgG)
- 📚 Check the [documentation](https://infrarust.dev)
