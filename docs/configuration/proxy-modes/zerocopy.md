---
title: Zero-Copy Mode
description: Linux-optimized proxy mode using splice(2) for kernel-level TCP forwarding without userspace copies.
---

# Zero-Copy Mode

Zero-copy mode behaves like [passthrough](./passthrough.md) but uses the Linux `splice(2)` syscall to move data between sockets through a kernel pipe. Bytes never get copied into userspace memory, which reduces CPU usage and can improve throughput on busy proxies.

## When to use it

Use zero-copy when you're running Infrarust on Linux and want lower CPU overhead than passthrough for high-traffic servers. It has the same limitations as passthrough (no packet inspection, no server switching), so it's best for single-server setups where raw performance matters.

On non-Linux systems, zero-copy falls back to the same `copy_bidirectional` forwarder that passthrough uses. You'll see a warning in the logs:

```
ZeroCopy mode requested but splice is only available on Linux, falling back to CopyForwarder
```

## Configuration

```toml
name = "my-server"
domains = ["mc.example.com"]
addresses = ["192.168.1.10:25565"]
proxy_mode = "zero_copy"
```

Or with Docker labels:

```yaml
labels:
  infrarust.domains: "mc.example.com"
  infrarust.proxy_mode: "zero_copy"
```

::: warning Linux only
The `splice(2)` syscall is a Linux kernel feature. On macOS, Windows, or other operating systems, zero-copy mode silently falls back to the standard copy forwarder. The behavior is identical to passthrough in that case.
:::

## How it works

1. The proxy completes the handshake and connects to the backend, same as passthrough.
2. It creates two kernel pipes (one per direction).
3. Each direction uses `splice(2)` to move data from one socket's kernel buffer into the pipe, then from the pipe into the other socket's kernel buffer.
4. Data never enters userspace. The kernel handles the transfer directly.

The default pipe size is 64 KiB. The forwarder uses nonblocking file descriptors and integrates with Tokio's readiness system to avoid busy-waiting.

## Constraints

Zero-copy is a forwarding mode, with the same constraints as passthrough:

- At least one domain is required.
- Cannot belong to a network (no server switching).
- No packet injection or inspection.
- Domain rewrite still works (it only affects the initial handshake, before the splice loop starts).

