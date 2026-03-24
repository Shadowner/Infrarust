# Quick Start Guide

This guide will help you install and configure Infrarust for your first use.

## Quick Installation

### Download Precompiled Binary

1. Download the latest version from the [releases page](https://github.com/shadowner/infrarust/releases)
2. Extract the archive to your desired location

## Basic Setup

### 1. Create Configuration Files

Create a `config.yaml` file in your working directory:

```yaml
# Minimal configuration
bind: "0.0.0.0:25565"  # Listening address
keepAliveTimeout: 30s
filters:
  rateLimiter:
    requestLimit: 10
    windowLength: 1s
```

Create a `proxies` folder and add server configurations:

```yaml
# proxies/my-server.yml
domains:
  - "hub.minecraft.example.com"  # Specific domain
addresses:
  - "localhost:25566"  # Minecraft server address
proxyMode: "passthrough"  # Proxy mode
```

### 2. Start Infrarust

```bash
./infrarust
```

### 3. Connect and Verify

1. Launch your Minecraft client
2. Connect to your configured domain
3. Check the logs to confirm the connection

## Folder Structure

```
infrarust/
├── config.yaml          # Main configuration
├── proxies/            # Server configurations
│   ├── hub.yml
│   └── survival.yml
├── infrarust[.exe]
└── logs/               # Logs (created automatically)
```

## Building from Source

If you prefer to build from source, you'll need:

- Rust 1.84 or higher
- Cargo (Rust package manager)

### Installation Methods

#### Via Cargo

```bash
cargo install infrarust
```

#### From Source

```bash
git clone https://github.com/shadowner/infrarust
cd infrarust
cargo build --release
```

To include Telemetry, you can use the `--features` flag when building:

```bash
cargo build --release --features telemetry
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
| `passthrough` | Direct transmission | No plugin functionality, just proxy compatible with every minecraft version |
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

## Next Steps

Once basic configuration is complete, you can:

1. [Configure different proxy modes](../proxy/modes/)
2. [Optimize performance](../proxy/performance)
3. [Configure monitoring](../quickstart/deployment.md)

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

- 🐛 Report a bug on [GitHub](https://github.com/shadowner/infrarust/issues)
- 💬 Join our [Discord](https://discord.gg/sqbJhZVSgG)

::: tip
Remember to check the documentation regularly as Infrarust is under active development and new features are added regularly.
:::
