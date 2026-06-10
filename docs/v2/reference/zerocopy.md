---
title: Zerocopy & Splice
description: How the zero_copy proxy mode forwards raw bytes with the Linux splice(2) syscall, why it is faster than a userspace copy, and what it gives up by never reading the packet stream.
---

# Zerocopy & Splice

The `zero_copy` proxy mode forwards a connection by moving bytes between the client socket and the backend socket inside the kernel, without copying them into Infrarust's address space. On Linux it does this with the `splice(2)` syscall. The mode is part of the forwarding family (`passthrough`, `zero_copy`, `server_only`): once the handshake is read and a backend is chosen, the rest of the connection is relayed byte for byte. Nothing reads or parses the packet stream after that point, which is exactly what makes the kernel path possible and what it costs you.

## Two forwarders

The transport layer (`crates/infrarust-transport/src/forward.rs`) has two strategies behind one `Forwarder` trait, and `select_forwarder(mode)` picks between them.

`CopyForwarder` is the portable one. It uses `tokio::io::copy_bidirectional`, which reads each direction into a userspace buffer and writes it back out. Every byte crosses the kernel boundary twice (read into the process, write out of it). This is the strategy used for every mode except `zero_copy`.

`SpliceForwarder` is the Linux-only one. It sets up a kernel pipe per direction and calls `splice(2)` to move data from the source socket into the pipe and from the pipe into the destination socket. The bytes never enter the process. There are two pipes, one for client to backend and one for backend to client, and each direction runs as its own splice loop. The default pipe size is 64 KiB (`SpliceForwarder::new()`); `with_pipe_size` lets a caller change it.

`select_forwarder` returns `SpliceForwarder` only for `ProxyMode::ZeroCopy` on a Linux build. Every other mode, and every non-Linux target, gets `CopyForwarder`.

## How splice forwarding works

Each direction loops over two steps. First it waits for the source socket to be readable and splices from the socket into that direction's pipe (a "drain"). Then it waits for the destination socket to be writable and splices from the pipe into the destination (a "pump"), repeating the pump until the whole drained chunk has been written. Readiness is taken from the existing tokio `TcpStream`, using `try_io` so the syscall runs without a second epoll registration. `EAGAIN` from the kernel is mapped to `WouldBlock` and retried rather than treated as an error.

When the source returns EOF (a splice of 0 bytes), that direction half-closes the destination's write side with `shutdown(Write)`. That EOF then propagates to the other direction, so the reverse loop drains whatever is left and finishes on its own. A `CancellationToken` covers a proxy shutdown: when it fires, both directions stop and the session ends with `ForwardEndReason::Shutdown`.

The forwarder reports a `ForwardResult` with the byte counts in each direction and the reason it ended (`ClientClosed`, `BackendClosed`, `Shutdown`, or `Error`).

## Linux only

`splice(2)` is a Linux syscall, so `SpliceForwarder` is compiled only under `#[cfg(target_os = "linux")]`. On any other platform the symbol does not exist and `select_forwarder` returns `CopyForwarder`.

If you set `proxy_mode = "zero_copy"` and run on Linux, you get the splice path. If you set it on macOS or Windows, the proxy does not refuse to start: config validation logs a warning (`proxy_mode = zero_copy is only supported on Linux`) and the connection is forwarded with the portable `CopyForwarder` instead. The behavior is correct either way, just not zero-copy off Linux.

::: tip
If you want the splice path, deploy on Linux. On other platforms `zero_copy` behaves like `passthrough` (a userspace copy) and the only signal is a startup warning, so do not assume the kernel path is active just because the mode is set.
:::

## What it precludes

The reason `zero_copy` is fast is the reason it is limited: after the handshake, Infrarust never sees the packet stream. It hands both sockets to the kernel and stops looking. That rules out everything that depends on reading or rewriting packets.

- No packet inspection. Once forwarding starts, the proxy reads nothing, so it cannot act on chat, plugin messages, or any in-stream event.
- No limbo. Holding a player on the proxy needs the proxy to speak the protocol to that player, which a raw relay cannot do.
- No server switching. Moving a player to another backend means re-driving login and play packets, and there is nothing in the byte stream the proxy is willing to read.
- No in-game commands on this path. The `/infrarust` command is intercepted by parsing chat packets, which a forwarding mode does not do.

In the plugin API these are the passive modes. A `Player` on a `zero_copy` (or `passthrough`) connection is not active, so `send_message`, `switch_server`, and similar methods return `PlayerError::NotActive`. See [In-game commands](./commands) for what the active modes expose, and [Config Schema](./config-schema) for the full `proxy_mode` list.

If you need any of those features, use an intercepted mode (`client_only` or `offline`). Those parse the packet stream and pay the userspace-copy cost in exchange for the ability to act on it.

## Choosing zero_copy

Reach for `zero_copy` when the backend handles authentication and play, and Infrarust is there to route by domain and move bytes as cheaply as possible. It is the lowest-overhead forwarding mode on Linux and a good fit for a status or routing front end where you do not need limbo, switching, or any packet-level feature.

```toml
# servers/survival.toml
domains = ["survival.mc.example.com"]
addresses = ["10.0.1.5:25565"]
proxy_mode = "zero_copy"
```

For the wider set of throughput knobs (worker threads, `SO_REUSEPORT`, and where `zero_copy` fits among them), see [Performance Tuning](./performance).
