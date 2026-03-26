---
title: Installation
description: Install Infrarust from a pre-built binary, Docker image, or build from source with Cargo.
---

# Installation

## Pre-built binaries

Download the latest release from the [GitHub Releases page](https://github.com/Shadowner/Infrarust/releases). Binaries are available for Linux (x86_64 and aarch64).

After downloading, make the binary executable and move it somewhere in your `PATH`:

```bash
chmod +x infrarust
sudo mv infrarust /usr/local/bin/
```

Verify the installation:

```bash
infrarust --help
```

## Docker

Infrarust ships a minimal Docker image built from `scratch` with a statically linked binary. The image exposes port `25565` and expects your configuration at `/app/config`.

```bash
docker run -d \
  --name infrarust \
  -p 25565:25565 \
  -v ./config:/app/config \
  ghcr.io/shadowner/infrarust:latest
```

Create a `config/` directory with your `infrarust.toml` and a `proxies/` subdirectory for server definitions before starting the container.

### Docker Compose

```yaml
services:
  infrarust:
    image: ghcr.io/shadowner/infrarust:latest
    ports:
      - "25565:25565"
    volumes:
      - ./config:/app/config
    restart: unless-stopped
```

### Building the image yourself

The included `Dockerfile` uses a multi-stage build with Alpine and produces a statically linked binary. It supports x86_64, aarch64, and armv7:

```bash
docker build -t infrarust .
```

## Build from source

### Requirements

- **Rust 1.85 or later** (edition 2024)
- A C linker (`gcc` or `clang`)
- OpenSSL development headers (for TLS support)

On Debian/Ubuntu:

```bash
sudo apt install build-essential pkg-config libssl-dev
```

On Alpine:

```bash
apk add musl-dev pkgconfig openssl-dev build-base
```

### Compile

Clone the repository and build the `infrarust` crate, which produces the `infrarust` binary:

```bash
git clone https://github.com/Shadowner/Infrarust.git
cd Infrarust
cargo build --release -p infrarust
```

The binary is at `target/release/infrarust`.

### Optional features

Enable features at compile time with `--features`:

| Feature | What it adds |
|---------|-------------|
| `telemetry` | OpenTelemetry tracing export |
| `plugin-auth` | Built-in authentication plugin |
| `plugin-hello` | Example hello-world plugin |
| `plugin-server-wake` | Wake-on-LAN / server start plugin |

```bash
cargo build --release -p infrarust --features telemetry,plugin-auth
```

### Static linking (musl)

For a fully static binary (no runtime dependencies), target musl. This is what the Docker image uses:

```bash
rustup target add x86_64-unknown-linux-musl
OPENSSL_STATIC=1 cargo build --release -p infrarust \
  --target x86_64-unknown-linux-musl
```

## CLI usage

```
infrarust [OPTIONS]
```

| Option | Default | Description |
|--------|---------|-------------|
| `-c, --config <PATH>` | `infrarust.toml` | Path to the proxy configuration file |
| `-b, --bind <ADDR>` | (from config) | Override the bind address, e.g. `0.0.0.0:25577` |
| `-l, --log-level <LEVEL>` | `info` | Log level filter (`trace`, `debug`, `info`, `warn`, `error`) |

The `RUST_LOG` environment variable takes priority over `--log-level` when both are set.

## Next steps

Once Infrarust is installed, head to [Quick Start](./quick-start.md) to set up your first proxy configuration.
