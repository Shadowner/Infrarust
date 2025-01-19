# Quick Start Guide

This guide will help you install and configure Infrarust for your first use.

## Prerequisites

Before starting, make sure you have:

> Those prequisite apply only if you don't use the precompiled binaries

- Rust 1.80 or higher
- Cargo (Rust package manager)
- An existing Minecraft server
- A domain (optional, for domain-based routing)

## Installation

### Method 1: Precompiled Binaries

Download the latest version from the [releases page](https://github.com/shadowner/infrarust/releases).

### Method 2: Via Cargo (Recommended)

```bash
cargo install infrarust
```

### Method 3: From Source

```bash
# Clone the repository
git clone https://github.com/shadowner/infrarust
cd infrarust

# Build the project
cargo build --release

# The executable is located in target/release/infrarust
```

## Quick Configuration

1. Create a `config.yaml` file in your working directory:

```yaml
# Minimal configuration
bind: "0.0.0.0:25565"  # Listening address
keepAliveTimeout: 30s
filters:
  rateLimiter:
    requestLimit: 10
    windowLength: 1s
keepalive_timeout: 30s  # Keepalive timeout
```

2. Create a `proxies` folder and add a configuration file for your server:

```yaml
# proxies/my-server.yml
domains:
  - "hub.minecraft.example.com"  # Specific domain
addresses:
  - "localhost:25566"  # Minecraft server address
proxyMode: "passthrough"  # Proxy mode
```

## Folder Structure

```
infrarust/
├── config.yaml          # Main configuration
├── proxies/            # Server configurations
│   ├── hub.yml
│   ├── survival.yml
│   └── creative.yml
├── infrarust.exe
└── logs/               # Logs (created automatically) //TODO: Not implemented yet
```

## First Steps

### 1. Start Infrarust

```bash
# If installed via cargo
infrarust --config-path "./custom_config_path/config.yaml" --proxies-path "./custom_proxies_path/" 

# If built from source
./target/release/infrarust --config-path "./custom_config_path/config.yaml" --proxies-path "./custom_proxies_path/" 
```

:::note
Argument needed only if the executable is not in the same repertory as depicted in the folder structure
:::

### 2. Verify Operation

1. Launch your Minecraft client
2. Connect to your configured domain
3. Check the logs to confirm the connection

## Available Proxy Modes

Infrarust offers several proxy modes for different use cases:

| Mode | Description | Use Case |
|------|-------------|----------|
| `passthrough` | Direct transmission | No plugin functionality, just proxy |
| `client_only` | Client-side auth | Servers in `online_mode=false`, but premium client |
| `offline` | No authentication | `online_mode=false` servers and cracked client |

> Other modes are under development

## Basic Configuration

### Simple DDoS Protection

```yaml
# In config.yaml
filters:
  rateLimiter:
    requestLimit: 10
    windowLength: 1s
```

### Status Cache

```yaml
# In config.yaml
statusCache: ### NOT IMPLEMENTED YET ###
  enabled: true
  ttl: 30s
```

## Next Steps

Once basic configuration is complete, you can:

1. [Configure different proxy modes](/proxy/modes)
2. [Optimize performance](/proxy/performance)
3. [Set up security](/proxy/security)
4. [Configure monitoring](/deployment/monitoring)

## Common Troubleshooting

### Proxy Won't Start

- Check if the port is already in use
- Make sure you have the necessary permissions
- Verify the configuration file syntax

### Clients Can't Connect

- Check domain configuration
- Ensure destination servers are accessible
- Check logs for specific errors
- Verify mode compatibility with your server

### Performance Issues

- Enable status cache
- Check rate limiter configuration
- Ensure your server has enough resources

## Need Help?

- 📖 Check the [complete documentation](/guide/)
- 🐛 Report a bug on [GitHub](https://github.com/shadowner/infrarust/issues)
- 💬 Join our [Discord](https://discord.gg/uzs5nZsWaB)
 
::: tip
Remember to check the documentation regularly as Infrarust is under active development and new features are added regularly.
:::
