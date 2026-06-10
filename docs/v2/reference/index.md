---
title: Reference
description: Technical reference for Infrarust configuration schema, CLI flags, in-game commands, error codes, benchmarking, and internal architecture.
---

# Reference

This section contains lookup material for Infrarust. Unlike the [Guide](/guide/) (which walks you through tasks) or [Configuration](/configuration/) (which explains how to set things up), reference pages are structured for quick lookup when you already know what you're looking for.

## Configuration

[Config Schema](./config-schema) documents every option in `infrarust.toml` and server TOML files: types, defaults, and valid values. If you need to check a field name or see what a default is, start here.

## CLI

[CLI Reference](./cli) covers the `infrarust` binary's command-line flags:

```bash
infrarust --config ./infrarust.toml --bind 0.0.0.0:25577 --log-level debug
```

The three flags are `--config` (path to the config file, defaults to `infrarust.toml`), `--bind` (override the listen address), and `--log-level` (log verbosity, defaults to `info`). The `RUST_LOG` environment variable takes priority over `--log-level`.

The CLI reference also documents the interactive console commands (for the operator running the terminal) and the bundled `registry-extractor` and `infrarust-stress-test` tools.

## In-game commands

[In-game commands](./commands) covers the `/infrarust` command (alias `/ir`) that players and admins type in Minecraft chat. The proxy intercepts these messages and never forwards them to the backend.

There are eleven subcommands grouped by permission level. Player-level subcommands (`help`, `version`, `list`, `find`, `server`) are visible to all players by default. Admin-only subcommands (`broadcast`, `kick`, `send`, `reload`, `plugin`, `plugins`) are hidden from players and require the `infrarust.admin` permission. Admins are listed under `[permissions]` in `infrarust.toml`.

## Error codes

[Error Codes](./error-codes) lists the error types Infrarust can produce during connection forwarding and server management, with likely causes and fixes.

## Proxy protocol

[Proxy protocol spec](./proxy-protocol) explains Infrarust's support for HAProxy PROXY protocol v1/v2, both receiving from upstream load balancers and sending to backend servers.

## Benchmarking

[Benchmarking](./benchmarking) describes the five-layer benchmark suite for Infrarust's intercepted-mode packet path. It covers the codec filter chain, frame codec, full CPU pipeline, end-to-end load measurement, and live per-filter timing, with the exact `cargo bench` commands to reproduce every number.

## Architecture

[Architecture Overview](./architecture) describes the internal structure of Infrarust: the crate layout, connection pipeline, plugin system, and how packets flow from client to backend.

[Performance Tuning](./performance) covers worker threads, `SO_REUSEPORT`, zero-copy mode, and other knobs for high-throughput deployments.

[Zerocopy & Splice](./zerocopy) explains the Linux `splice(2)` forwarding path used by the `zero_copy` proxy mode.

## Migration

[Migration from V1](./migration-v1) maps V1 configuration options to their V2 equivalents for anyone upgrading from a previous Infrarust release.
