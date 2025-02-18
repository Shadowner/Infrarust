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
keepalive_timeout: 30s          # Connection keepalive timeout
domains: ["example.com"]        # Default domains (optional)
addresses: ["localhost:25566"]  # Default target addresses (optional)

# File Provider Configuration
file_provider:
  proxies_path: ["./proxies"]   # Path to proxy configurations
  file_type: "yaml"            # File type (currently only yaml supported)
  watch: true                  # Enable hot-reload of configurations

# Telemetry Configuration
telemetry:
  enabled: false               # Enable telemetry collection
  export_interval_seconds: 30  # Export interval
  export_url: "http://..."    # Export destination (optional)
  enable_metrics: false       # Enable metrics collection
  enable_tracing: false      # Enable distributed tracing

# Default MOTD Configuration
motds:
  unknown:                    # MOTD for unknown servers
    version: "1.20.1"        # Minecraft version to display
    max_players: 100         # Maximum players to show
    online_players: 0        # Online players to show
    description: "Unknown server" # Server description
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

proxyMode: "passthrough"    # Proxy mode (passthrough/client_only/offline)

# Filter Configuration
filters:
  rate_limiter:
    requestLimit: 10        # Maximum requests per window
    windowLength: 1s        # Time window for rate limiting
  ip_filter:                # Possible to set but NOT IMPLEMENTED YET
    enabled: true
    whitelist: ["127.0.0.1"]
    blacklist: []
  id_filter:                # Possible to set but NOT IMPLEMENTED YET
    enabled: true
    whitelist: ["uuid1", "uuid2"]
    blacklist: []
  name_filter:             # Possible to set but NOT IMPLEMENTED YET
    enabled: true
    whitelist: ["player1"] 
    blacklist: []

# MOTD Configuration (overrides default / server motd)
motd:
  version: "1.20.1"       # Can be set to any text
  max_players: 100
  online_players: 0
  description: "Welcome to my server!"
  favicon: "data:image/png;base64,..."
```

## Feature Reference

### Proxy Modes

| Mode | Description |
|------|-------------|
| `passthrough` | Direct proxy, compatible with all Minecraft versions |
| `client_only` | For premium clients connecting to offline servers |
| `offline` | For offline clients and servers |

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
  description: "Text"      # Server description
  favicon: "base64..."     # Server icon (base64 encoded PNG)
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

## Advanced Features

### Hot Reload

When `file_provider.watch` is enabled, configuration changes are automatically detected and applied without restart.

> Active by default

### Not Implemented Features

The following features are planned but not yet implemented:

- Environment variable substitution
- Status cache
- Advanced DDoS protection at proxy level
- Advanced logging configuration
- Performance tuning options
- Configuration validation command
- Advanced load balancing
- Server-specific player limits
- Compression fine-tuning
- Multiple providers (docker, k8s...)

## Need Help?

- 🐛 Report issues on [GitHub](https://github.com/shadowner/infrarust/issues)
- 💬 Join our [Discord](https://discord.gg/uzs5nZsWaB)
- 📚 Check the [documentation](https://infrarust.dev)
