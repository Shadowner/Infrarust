---
title: Installation
description: Install Infrarust from a pre-built binary, Docker image, or build from source with Cargo.
---

# Installation

## Pre-built binaries

Each release on the [GitHub Releases page](https://github.com/Shadowner/Infrarust/releases) carries one archive per platform:

| Archive | Platform | Target |
|---------|----------|--------|
| `infrarust-linux-x86_64.tar.gz` | Linux, x86_64 | `x86_64-unknown-linux-musl` |
| `infrarust-linux-arm64.tar.gz` | Linux, aarch64 | `aarch64-unknown-linux-musl` |
| `infrarust-darwin-x86_64.tar.gz` | macOS, Intel | `x86_64-apple-darwin` |
| `infrarust-darwin-arm64.tar.gz` | macOS, Apple silicon | `aarch64-apple-darwin` |
| `infrarust-windows-x86_64.zip` | Windows, x86_64 | `x86_64-pc-windows-msvc` |
| `infrarust-windows-arm64.zip` | Windows, ARM64 | `aarch64-pc-windows-msvc` |

Each archive holds a single file at its root: `infrarust`, or `infrarust.exe` on Windows. The Linux binaries are built for musl and statically linked, so they do not depend on the system's C library.

Download the archive for your platform, extract it, and move the binary somewhere in your `PATH`:

```bash
curl -LO https://github.com/Shadowner/Infrarust/releases/latest/download/infrarust-linux-x86_64.tar.gz
tar xzf infrarust-linux-x86_64.tar.gz
sudo mv infrarust /usr/local/bin/
```

The tar archives keep the executable bit, so there is nothing to `chmod`. On Windows, extract the zip and run `infrarust.exe`.

A release also carries `manifest.json`, its signature `manifest.json.minisig`, and `SHA256SUMS`. [Upgrading](./upgrading#verify-a-release-by-hand) shows how to check an archive against them.

Verify the installation:

```bash
infrarust --help
```

## Docker

Infrarust ships a minimal Docker image built from `scratch` with a statically linked binary. The image exposes ports `25565` (proxy) and `8080` (admin API), and expects your configuration at `/app/config`.

```bash
docker run -d \
  --name infrarust \
  -p 25565:25565 \
  -v ./config:/app/config \
  ghcr.io/shadowner/infrarust:latest
```

Create a `config/` directory with your `infrarust.toml`, a `servers/` subdirectory for server definitions, and a `plugins/` subdirectory before starting the container.

### Docker Compose

```yaml
services:
  infrarust:
    image: ghcr.io/shadowner/infrarust:latest
    ports:
      - "25565:25565"
      - "8080:8080"
    volumes:
      - ./config:/app/config
    restart: unless-stopped
```

### Building the image yourself

The included `Dockerfile` uses a multi-stage build with Alpine and produces a statically linked binary. It also builds the admin frontend with Node.js. Supported architectures are x86_64, aarch64, and armv7:

```bash
docker build -t infrarust .
```

## Build from source

### Requirements

- Rust 1.99 or later (edition 2024)
- A C compiler and linker (`gcc` or `clang`). Several dependencies compile C code, among
  them `libdeflate` for packet compression and the `mimalloc` allocator.
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

| Feature | Default | What it adds |
|---------|---------|-------------|
| `libdeflater` | yes | libdeflate packet compression |
| `default-plugins` | yes | Bundles `plugin-auth` and `plugin-server-wake` |
| `plugin-auth` | via `default-plugins` | Built-in authentication plugin |
| `plugin-server-wake` | via `default-plugins` | Holds players in limbo while their target server starts up |
| `wasm` | yes | WASM plugin loader |
| `mimalloc` | yes | mimalloc as the memory allocator |
| `plugin-hello` | no | Example hello-world plugin |
| `docker` | no | Docker integration for managing backend containers |
| `telemetry` | no | OpenTelemetry tracing export |

```bash
cargo build --release -p infrarust --features telemetry,plugin-auth
```

Without the `libdeflater` feature, the proxy uses the pure-Rust flate2 backend for packet
compression. `--no-default-features` turns off every default, so list the ones you keep:

```bash
cargo build --release -p infrarust --no-default-features --features default-plugins,wasm,mimalloc
```

Other dependencies still compile C code, so the build needs a C compiler either way.

### Static linking (musl)

For a fully static binary (no runtime dependencies), target musl. This is what the Docker image uses:

```bash
rustup target add x86_64-unknown-linux-musl
OPENSSL_STATIC=1 cargo build --release -p infrarust \
  --target x86_64-unknown-linux-musl
```

## CLI usage

```
infrarust [OPTIONS] [COMMAND]
```

| Option | Default | Description |
|--------|---------|-------------|
| `-c, --config <PATH>` | `infrarust.toml` | Path to the proxy configuration file |
| `-b, --bind <ADDR>` | (from config) | Override the bind address, e.g. `0.0.0.0:25577` |
| `-l, --log-level <LEVEL>` | `info` | Log level filter (`trace`, `debug`, `info`, `warn`, `error`) |
| `--plugins-dir <PATH>` | (from config) | Override the plugins directory path |
| `--servers-dir <PATH>` | (from config) | Override the servers directory path |

The `RUST_LOG` environment variable takes priority over `--log-level` when both are set.

Without a command, Infrarust starts the proxy. The commands run a single task and exit:

| Command | What it does |
|---------|--------------|
| `migrate` | Converts V1 YAML configs to the V2 TOML format |
| `check-config` | Validates the proxy configuration without starting the proxy |
| `update` | Checks for, installs or rolls back a release |

Options go before the command, as in `infrarust -c /etc/infrarust/infrarust.toml check-config`. The [CLI reference](../reference/cli) lists the options of each command.

## Next steps

Once Infrarust is installed, head to [Quick Start](./quick-start) to set up your first proxy configuration.
