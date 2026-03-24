---
title: Zero-Copy Mode
description: Linux-optimized proxy mode using splice(2) for kernel-level TCP forwarding without userspace copies.
outline: [2, 3]
---

# Zero-Copy Mode

Zero-copy mode behaves like [passthrough](./passthrough.md) but uses the Linux `splice(2)` syscall to move data between sockets through kernel pipes. Bytes never get copied into userspace memory. On busy proxies, this reduces CPU usage compared to passthrough.

## When to use it

Use zero-copy when you're running Infrarust on Linux and want lower CPU overhead for high-traffic servers. It has the same limitations as passthrough: no packet inspection, no server switching, no plugin packet injection.

On non-Linux systems, zero-copy falls back to the same `CopyForwarder` that passthrough uses. The logs will tell you:

```
WARN ZeroCopy mode requested but splice is only available on Linux, falling back to CopyForwarder
```

::: warning Linux only
`splice(2)` is a Linux kernel feature. On macOS, Windows, or FreeBSD, setting `proxy_mode = "zero_copy"` gives you passthrough behavior with an extra log warning. If you deploy on mixed platforms, this is safe but pointless on non-Linux hosts.
:::

## Configuration

```toml
domains = ["mc.example.com"]
addresses = ["192.168.1.10:25565"]
proxy_mode = "zero_copy" // [!code focus]
```

With Docker labels:

```yaml
labels:
  infrarust.domains: "mc.example.com"
  infrarust.proxy_mode: "zero_copy"
```

All the same options from [passthrough](./passthrough.md#config-options) apply: `send_proxy_protocol`, `domain_rewrite`, `timeouts`, `max_players`, `disconnect_message`.

### Full example

```toml
name = "pvp"
domains = ["pvp.mc.example.com"]
addresses = ["10.0.1.20:25565"]
proxy_mode = "zero_copy"
send_proxy_protocol = true
max_players = 200

[timeouts]
connect = "3s"
read = "30s"
write = "30s"
```

## How it works

The handshake and backend connection work exactly like passthrough. The difference is what happens after the initial packets are forwarded.

### Passthrough (CopyForwarder)

Two tokio tasks call `tokio::io::copy`, reading bytes from one socket into a userspace buffer and writing them to the other socket. Every byte crosses the kernel-userspace boundary twice per direction.

```
Client socket → kernel → userspace buffer → kernel → Backend socket
```

### Zero-copy (SpliceForwarder)

Two async loops call `splice(2)`, moving bytes from one socket's kernel buffer into a kernel pipe, then from the pipe into the other socket's kernel buffer. Bytes stay in kernel memory the entire time.

```
Client socket → kernel pipe → Backend socket
```

Each direction gets its own kernel pipe. The flow for one direction:

1. `splice(source_fd, pipe_write_fd)` drains data from the source socket into the pipe. Chunk size is 65,536 bytes per call.
2. `splice(pipe_read_fd, dest_fd)` pumps data from the pipe into the destination socket. This loop runs until all drained bytes are written.
3. When the source socket reaches EOF, the destination's write half is shut down via `shutdown(fd, SHUT_WR)` to propagate the close signal.

The forwarder uses `SPLICE_F_NONBLOCK` and `SPLICE_F_MOVE` flags, and integrates with tokio's readiness system (`TcpStream::ready()` + `try_io()`) to avoid busy-waiting.

### Pipe size

Each kernel pipe is allocated at 64 KiB by default. The forwarder attempts to set this size via `F_SETPIPE_SZ`, but the kernel may cap it based on `/proc/sys/fs/pipe-max-size`. The default Linux limit is 1 MiB, so 64 KiB always succeeds unless you've lowered the system limit.

There is no config file option to change the pipe size. The default works well for Minecraft traffic, which sends small packets frequently rather than large bulk transfers.

## Performance: zero-copy vs passthrough

Both modes do the same thing from the player's perspective. The difference is where the byte copying happens.

| | Passthrough | Zero-copy |
|---|---|---|
| Copy location | Userspace (tokio buffer) | Kernel (pipe buffer) |
| Kernel-userspace crossings per byte | 2 per direction | 0 |
| CPU overhead | Low | Lower |
| Memory overhead | Tokio-managed buffers | Two 64 KiB kernel pipes per session |
| Platform | All | Linux only |
| Latency | Negligible difference | Negligible difference |

For a single Minecraft server with a few dozen players, the difference is not measurable. The savings become visible when you're proxying hundreds of concurrent sessions and want to squeeze more out of the same hardware.

::: tip
If you're unsure whether zero-copy helps your setup, start with passthrough. Switch to zero-copy if you observe high CPU usage from the proxy process on a Linux host. The configuration change is a single line.
:::

### Kernel resource usage

Each zero-copy session allocates two kernel pipes (one per direction). A pipe uses 64 KiB of kernel memory by default. For 1,000 concurrent sessions, that's about 128 MiB of kernel pipe buffers. This is well within normal limits on any server with a few gigabytes of RAM.

The pipe file descriptors are cleaned up when the session ends. If the kernel can't allocate a pipe (extremely unlikely), the session fails with an I/O error.

## Constraints

Zero-copy is a forwarding mode, with the same constraints as passthrough:

- At least one domain is required for routing.
- Cannot belong to a network (no server switching).
- No packet injection. Plugins that need to send packets to the player won't work.
- Works with every Minecraft version (1.7+).
- Domain rewrite still works. It only affects the initial handshake, before the splice loop starts.

## Compared to other modes

- Need cross-platform support? Use [passthrough](./passthrough.md). Same behavior, works everywhere.
- Need server switching or plugins? Use [client-only](./client-only.md). The proxy handles Mojang auth and can move players between backends.
- Need server switching without authentication? Use [offline](./offline.md).
