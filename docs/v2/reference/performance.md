---
title: Performance Tuning
description: "Tuning knobs for high-throughput Infrarust deployments: worker threads, zero-copy forwarding, the libdeflate backend, SO_REUSEPORT, TCP keepalive, the status cache, and connection limits."
---

# Performance Tuning

Infrarust runs fast on its defaults, so most deployments need no tuning at all. This page is for the cases where you are pushing a lot of connections through one process, or you want to understand which knob affects which cost. Every option here lives in `infrarust.toml` (the [config reference](./config-schema) has the full schema) or in the build itself.

Before tuning, know which path your traffic takes. The forwarding modes (`passthrough`, `zero_copy`, `server_only`) relay raw bytes after the handshake and never decode packets, so their per-connection cost is dominated by the kernel and the runtime, not by Infrarust. The intercepted modes (`client_only` and `offline`) decode every packet; unmodified packets are then forwarded as their original wire bytes, and only the ones a filter or the proxy changes are re-encoded and re-compressed, which is where compression and the codec chain matter. The [benchmarking](./benchmarking) page measures both.

## Worker threads

The async runtime is a multi-threaded tokio runtime. By default `worker_threads` is `0`, which means tokio picks the worker count itself (one per available CPU core). Set it to a fixed number only when you want to cap how many cores the proxy uses, for example to leave headroom for a backend running on the same host.

```toml
worker_threads = 4
```

A value of `0` is the right choice on a dedicated proxy host. Infrarust only overrides tokio's default when `worker_threads` is greater than zero, so setting it to `0` and leaving it out are equivalent. Oversubscribing (more workers than cores) rarely helps a network-bound proxy and adds scheduler overhead, so keep the count at or below your core count.

## SO_REUSEPORT

`so_reuseport` is off by default. When enabled it sets the `SO_REUSEPORT` socket option on the listener, which lets the kernel load-balance incoming connections across multiple sockets bound to the same address. This matters when you run several Infrarust processes on one host, each binding the same port. The kernel then spreads accepts across them instead of funneling every connection through one listener.

```toml
so_reuseport = true
```

This is a Linux-only option. On other platforms the flag parses fine but has no effect, because the socket code only calls `set_reuse_port` under `target_os = "linux"`. `SO_REUSEADDR` is always set regardless of this flag, and the listen backlog is fixed at 1024.

A single Infrarust process already accepts on all its worker threads, so `SO_REUSEPORT` is a multi-process knob, not a single-process one. Reach for it when one process cannot keep up and you want to fan out across a process pool.

## Zero-copy forwarding

The `zero_copy` proxy mode forwards raw TCP using the Linux `splice(2)` syscall, moving bytes between the client and backend sockets through a kernel pipe without copying them into userspace. The default forwarder, used by `passthrough` and `server_only`, is a portable userspace copy built on `tokio::io::copy_bidirectional`. Both relay bytes unchanged after the handshake; the difference is purely where the copy happens.

```toml
# in a server TOML
proxy_mode = "zero_copy"
```

`zero_copy` is Linux-only. The forwarder selection falls back to the userspace copy on any other platform and logs a warning, so a `zero_copy` server on macOS or Windows still works, just without the splice path. Each splice direction uses a 64 KiB kernel pipe.

Because `zero_copy` does no packet inspection, it cannot do server switching, limbo, or anything that needs to read Play packets. Use it for stable routes where the proxy only needs to move bytes. The [zero-copy and splice](./zerocopy) page explains the forwarding path in detail.

::: tip
Pick the mode by what the route needs, not by raw speed. A `zero_copy` route is the cheapest per byte, but if you need limbo or live server switching you want an intercepted mode (`client_only` or `offline`) and should tune compression instead.
:::

## Compression backend (libdeflate)

In the intercepted modes, compression is the single largest per-packet cost once a packet crosses the compression threshold. Infrarust selects the zlib backend at compile time. The shipped `infrarust` binary enables the `libdeflater` feature by default, which uses libdeflate (a C library) and runs roughly 2-3x faster than the pure-Rust path.

Building with `--no-default-features` drops back to `flate2` (pure Rust via miniz_oxide), which needs no C toolchain but is slower. Unless you have a reason to avoid the C dependency, keep the default.

```bash
# default build: libdeflate backend
cargo build --release -p infrarust

# pure-Rust fallback, no C toolchain needed
cargo build --release -p infrarust --no-default-features
```

The two backends use different compression level scales (flate2 is 0-9, libdeflate is 1-12), so a given numeric level is not equivalent across them. The forwarding modes never compress, so this knob only affects intercepted traffic.

## TCP keepalive

Infrarust sets TCP keepalive on every accepted connection so dead peers are detected and cleaned up rather than lingering. The `[keepalive]` table controls the probe timing.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `time` | duration | `"30s"` | Idle time before the first probe |
| `interval` | duration | `"10s"` | Interval between probes |
| `retries` | integer | `3` | Failed probes before the connection is dropped |

```toml
[keepalive]
time = "30s"
interval = "10s"
retries = 3
```

`retries` takes effect on Linux and macOS only; other platforms ignore it because the socket layer does not expose it there. `time` and `interval` apply everywhere. `TCP_NODELAY` is also set on every accepted connection, so small packets are sent without Nagle delay; this is not configurable.

## Status cache

Status pings (the server list ping a client sends before connecting) are cheap to serve but easy to flood, since a client can ping without authenticating. The `[status_cache]` table caches the response so repeated pings for the same target do not hit the backend each time.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `ttl` | duration | `"5s"` | How long a cached status response stays valid |
| `max_entries` | integer | `1000` | Maximum number of cached responses |

```toml
[status_cache]
ttl = "5s"
max_entries = 1000
```

A longer `ttl` reduces backend load under heavy ping traffic at the cost of staler player counts and MOTDs. For rate limiting the ping volume itself, the separate `status_max` and `status_window` keys under `[rate_limit]` cap how many status connections a single IP can open per window (default 300 per 10 seconds).

## Connection limits

`max_connections` caps the number of simultaneous connections the proxy accepts. The default is `0`, which means unlimited. When set above zero, the accept loop holds a semaphore permit for the lifetime of each connection and will not accept a new one until a permit is free, so the cap is a hard ceiling rather than a soft target.

```toml
max_connections = 5000
```

Set this as a backstop against resource exhaustion (file descriptors, memory) rather than as a routine limit. Per-IP login flood protection is a different concern, handled by `[rate_limit]` with `max_connections` and `window` keys scoped to each IP.

## A note on load balancing

Listing several `addresses` for one server spreads sessions across them. The default strategy is `first_available`, which keeps the old sequential failover behavior; switch to `round_robin` or `least_conn` to actually distribute load. Per-address connection counting is O(1), so it costs nothing measurable per login. See [Load balancing](../configuration/load-balancing).

## Where to measure

Tuning without measuring is guessing. The [benchmarking](./benchmarking) page documents the benchmark suite and the `cargo bench` commands behind every number, including the compression cliff that the libdeflate backend addresses and the per-filter timing you can turn on in a running proxy. Measure your own hardware before and after a change; the example numbers in that page are from one developer machine and are shapes, not targets.
