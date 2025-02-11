# Installation Guide

This guide details all methods for installing Infrarust across different operating systems and environments.

## Table of Contents

[[toc]]

## System Requirements

### Minimum Hardware

- CPU: 1 core
- RAM: 256 MB
- Storage: 100 MB

### Recommended Hardware

- CPU: 2 cores or more for high traffic
- RAM: 1 GB or more for high traffic
- Storage: 250 MB

### Required Software

- Rust 1.80+
- Git (for source installation)
- Compatible operating system:
  - Linux (kernel 3.17+)
  - Windows 10/11
  - macOS 10.15+

## Installation via Cargo

The simplest method to install Infrarust is using Cargo, the Rust package manager.

### 1. Installing Rust and Cargo

```bash
# On Linux and macOS
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# On Windows
# Download and run rustup-init.exe from https://rustup.rs/
```

### 2. Installing Infrarust

```bash
cargo install infrarust
```

## Installation from Source

This method provides the latest version and allows customizing the build.

### 1. Clone the Repository

```bash
git clone https://github.com/shadowner/infrarust
cd infrarust
```

### 2. Building

```bash
# Build in release mode
cargo build --release

# The executable will be in
# target/release/infrarust
```

## Installation via Precompiled Binaries

### Linux

```bash
# Download
curl -LO https://github.com/shadowner/infrarust/releases/latest/download/infrarust-linux-x86_64.tar.gz

# Extract
tar xzf infrarust-linux-x86_64.tar.gz

# Move to PATH
sudo mv infrarust /usr/local/bin/
```

### Windows

1. Download the ZIP file from the [releases page](https://github.com/shadowner/infrarust/releases)
2. Extract the contents
3. Add the folder to system PATH or use the full path

### macOS

```bash
# Download
curl -LO https://github.com/shadowner/infrarust/releases/latest/download/infrarust-macos-x86_64.tar.gz

# Extract
tar xzf infrarust-macos-x86_64.tar.gz

# Move to PATH
sudo mv infrarust /usr/local/bin/
```

## Installation via Docker

### Using the Official Image

```bash
docker pull shadowner/infrarust:latest
```

### Docker Compose

```yaml
version: "3.8"

services:
  infrarust:
    image: shadowner/infrarust:latest
    container_name: infrarust
    restart: always
    ports:
      - "25565:25565"
    volumes:
      - ./config.yaml:/etc/infrarust/config.yaml
      - ./proxies:/etc/infrarust/proxies
```

## Development Installation

If you want to contribute to development:

```bash
# Clone with submodules
git clone --recursive https://github.com/shadowner/infrarust
cd infrarust

# Build in development mode
cargo build

# Run tests
cargo test
```

## Post-Installation Setup

### Linux: Systemd Service

Create a service file:

```ini
# /etc/systemd/system/infrarust.service
[Unit]
Description=Infrarust Minecraft Proxy
After=network.target

[Service]
Type=simple
User=minecraft
ExecStart=/usr/local/bin/infrarust
WorkingDirectory=/opt/infrarust
Restart=always

[Install]
WantedBy=multi-user.target
```

Enable the service:

```bash
sudo systemctl enable infrarust
sudo systemctl start infrarust
```

## Troubleshooting

### Common Errors

1. **Compilation Error**

   ```
   Solution: Update Rust with 'rustup update'
   ```

2. **Port Already in Use**

   ```
   Solution: Change port in config.yaml or free port 25565
   ```

3. **Insufficient Permissions**

   ```
   Solution: Run with sudo or as administrator
   ```

## Updating

### Via Cargo

```bash
cargo install infrarust --force
```

### From Source

```bash
git pull
cargo build --release
```

### Via Docker

```bash
docker pull shadowner/infrarust:latest
```

::: tip
For production environments, it's recommended to use a specific version rather than latest.
:::

## Support

If you encounter installation issues:

1. Check [known issues](https://github.com/shadowner/infrarust/issues)
2. Join our [Discord](https://discord.gg/sqbJhZVSgG)
3. Open a ticket on GitHub
