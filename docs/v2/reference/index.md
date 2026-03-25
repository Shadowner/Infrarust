---
title: Reference
description: Technical reference for Infrarust configuration schema, CLI flags, error codes, and internal architecture.
---

# Reference

This section contains lookup material for Infrarust. Unlike the [Guide](/guide/) (which walks you through tasks) or [Configuration](/configuration/) (which explains how to set things up), reference pages are structured for quick lookup when you already know what you're looking for.

## Configuration

[Config Schema](./config-schema.md) documents every option in `infrarust.toml` and server TOML files: types, defaults, and valid values. If you need to check a field name or see what a default is, start here.

## CLI

[CLI Reference](./cli.md) covers the `infrarust` binary's command-line flags:

```bash
infrarust --config ./infrarust.toml --bind 0.0.0.0:25577 --log-level debug
```

The three flags are `--config` (path to the config file, defaults to `infrarust.toml`), `--bind` (override the listen address), and `--log-level` (log verbosity, defaults to `info`). The `RUST_LOG` environment variable takes priority over `--log-level`.

## Error Codes

[Error Codes](./error-codes.md) lists the error types Infrarust can produce during connection forwarding and server management, with likely causes and fixes.

## Proxy Protocol

[Proxy Protocol Spec](./proxy-protocol.md) explains Infrarust's support for HAProxy PROXY protocol v1/v2, both receiving from upstream load balancers and sending to backend servers.

## Architecture

[Architecture Overview](./architecture.md) describes the internal structure of Infrarust: the crate layout, connection pipeline, plugin system, and how packets flow from client to backend.

[Performance Tuning](./performance.md) covers worker threads, `SO_REUSEPORT`, zero-copy mode, and other knobs for high-throughput deployments.

[Zerocopy & Splice](./zerocopy.md) explains the Linux `splice(2)` forwarding path used by the `zero_copy` proxy mode.

## Migration

[Migration from V1](./migration-v1.md) maps V1 configuration options to their V2 equivalents for anyone upgrading from a previous Infrarust release.
