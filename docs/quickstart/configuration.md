# Infrarust Configuration

This guide details all configuration options available in Infrarust.

:::warning
**Note:** This documentation may not reflect all currently implemented features. Any unavailable features will be clearly marked as such throughout the documentation.
:::

## Configuration File Structure

Infrarust uses two types of configuration files:

```
infrarust/
├── config.yaml         # Global configuration
└── proxies/           # Server configurations
    ├── hub.yml
    ├── survival.yml
    └── creative.yml
```

## Global Configuration

The `config.yaml` file contains Infrarust's main configuration.

### Minimal Configuration

```yaml
bind: "0.0.0.0:25565"
domains: ### NOT IMPLEMENTED YET ###
  - "*.minecraft.example.com"
keepalive_timeout: 30s
```

### Complete Configuration

```yaml
# Proxy listening address
bind: "0.0.0.0:25565"

# List of accepted domains
domains: ### NOT IMPLEMENTED YET ###
  - "*.minecraft.example.com"
  - "play.example.com"

# Timeout settings
keepalive_timeout: 30s
read_timeout: 30s ### NOT IMPLEMENTED YET ###
write_timeout: 30s ### NOT IMPLEMENTED YET ###

# Cache configuration
status_cache: ### NOT IMPLEMENTED YET ###
  enabled: true
  ttl: 30s
  max_size: 1000

# Security
security: ### NOT IMPLEMENTED YET (only for proxy level not server level) ###
  # DDoS protection
  rate_limiter:
    enabled: true
    requests: 10
    window: "1s"
  
  # IP filtering
  ip_filter: ### NOT IMPLEMENTED YET ###
    enabled: true
    blacklist:
      - "1.2.3.4"
      - "10.0.0.0/8"
    whitelist:
      - "192.168.1.0/24"

# Logging configuration
logging: ### NOT IMPLEMENTED YET ###
  level: "info"  # debug, info, warn, error
  file: "logs/infrarust.log"
  format: "json"  # json, text

# Performance configuration
performance: ### NOT IMPLEMENTED YET ###
  workers: 4
  connection_pool_size: 100
  buffer_size: 8192
```

## Proxy Server Configuration

Each file in the `proxies/` folder represents a Minecraft server.

### Simple Configuration

```yaml
# proxies/hub.yml
domains:
  - "hub.minecraft.example.com"
addresses:
  - "localhost:25566"
proxy_mode: "passthrough"
```

### Advanced Configuration

:::warning
**Note:** Not all features are implemented yet, but this configuration file represents one of the development goals
[#1 Roadmap](https://github.com/Shadowner/Infrarust/issues/1)
:::

```yaml
# Complete server configuration
name: "Main Hub"

# Accepted domains
domains:
  - "hub.minecraft.example.com"
  - "*.hub.minecraft.example.com"

# Backend server addresses
addresses:
  - "localhost:25566"
  - "localhost:25567"  # Failover  ### NOT IMPLEMENTED YET ###

# Proxy mode
proxy_mode: "clientOnly"  # passthrough, clientOnly, offline

# Authentication configuration
authentication: ### NOT IMPLEMENTED YET ###
  enabled: true
  timeout: 30s
  cache_time: 300s

# Proxy protocol configuration
proxy_protocol: ### NOT IMPLEMENTED YET ###
  enabled: true
  version: 2
  trusted_proxies:
    - "10.0.0.0/8"

# Load balancing
load_balancing: ### NOT IMPLEMENTED YET ###
  method: "round_robin"  # round_robin, least_conn, random
  health_check:
    enabled: true
    interval: 10s
    timeout: 5s
    unhealthy_threshold: 3

# Server limits
limits: ### NOT IMPLEMENTED YET ###
  max_players: 1000
  max_connections_per_ip: 3

# MOTD configuration
motd: ### NOT IMPLEMENTED YET ###
  enabled: true
  custom_text: "§6§lMain Hub §r- §aOnline"
  max_players: 1000
  
# Compression
compression: ### NOT IMPLEMENTED YET ###
  threshold: 256
  level: 6
```

## Environment Variables - Not implemented

Infrarust supports configuration via environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `INFRARUST_CONFIG` | Config file path | `config.yaml` |
| `INFRARUST_PROXIES_DIR` | Proxies directory | `proxies` |
| `INFRARUST_LOG_LEVEL` | Log level | `info` |
| `INFRARUST_BIND` | Listening address | `0.0.0.0:25565` |

## Proxy Modes

### Proxy Modes

Infrarust supports different proxy modes that can be configured per server. See [Proxy Modes](/proxy/) for detailed information.

| Mode | Description |
|------|-------------|
| `passthrough` | Direct connection pass-through |
| `clientOnly` | Client-side validation only |
| `offline` | Offline mode operation |

## Advanced Options

### TLS Configuration

```yaml
tls: ### NOT IMPLEMENTED YES ###
  enabled: true
  cert_file: "cert.pem"
  key_file: "key.pem"
  min_version: "1.2"
```

### Metrics and Monitoring - Not implemented

```yaml
metrics: ### NOT IMPLEMENTED YET ###
  enabled: true
  bind: "127.0.0.1:9100"
  path: "/metrics"
  type: prometheus
```

## Configuration Validation

To validate your configuration:

```bash
infrarust validate --config config.yaml ### NOT IMPLEMENTED YET ###
```

::: tip
Timeout values accept suffixes: "s" (seconds), "m" (minutes), "h" (hours).

```yaml
keepalive_timeout: 30s
read_timeout: 1m ### NOT IMPLEMENTED YET ###
write_timeout: 2h ### NOT IMPLEMENTED YET ###
```
:::

::: warning
Configuration changes require a proxy restart unless hot reloading is enabled.
:::
