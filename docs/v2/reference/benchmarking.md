---
title: Benchmarking
description: How Infrarust's intercepted-mode packet path is benchmarked (per-packet latency, per-plugin overhead, and end-to-end load), with the commands to reproduce every number.
---

# Benchmarking

Infrarust's intercepted modes (`client_only`, `offline`) decode every packet and run it
through the codec filter chain. Unmodified frames are then forwarded as their original
wire bytes; only packets that a filter or the proxy modifies are re-encoded (and
re-compressed). This page covers how that hot path is measured, so you can answer two
questions with real numbers:

1. How long does a packet take to cross the proxy in intercepted mode?
2. How much time does each plugin/filter add ("timing between plugins")?

The suite is layered. Each layer isolates one cost, so the numbers stay attributable.
The cheap layers are deterministic CPU microbenchmarks; the top layer is end-to-end
network latency under sustained load.

Passthrough and zero-copy modes do none of this per-packet work; `CopyForwarder` and
`SpliceForwarder` relay raw TCP. The gap between them and intercepted mode is the
proxy-added cost, which Layer D measures directly.

## Tooling

| Tool | Used for |
|------|----------|
| [divan](https://github.com/nvzqz/divan) | sub-microsecond microbenchmarks (Layers A to C), `harness = false` |
| [hdrhistogram](https://github.com/HdrHistogram/HdrHistogram_rust) | low-overhead latency recording in the load tool (Layer D) |
| [quanta](https://github.com/metrics-rs/quanta) | cheap TSC timestamps for the `bench-timing` feature (Layer E) |
| tracing | per-filter spans on the `infrarust::bench_timing` target (Layer E) |

Every example number below comes from one developer machine, so your hardware will
differ. Read them as shapes and ratios, not absolutes.

## Layer A: codec filter chain

`cargo bench -p infrarust-core --bench codec_chain`

Times `CodecFilterChain::process`, the sync call every codec plugin runs through.

`filter_overhead` sweeps the chain length (0, 1, 2, 4, 8 passthrough filters) at a fixed
payload. The delta between successive counts is the framework's per-plugin dispatch cost,
independent of what the plugin does. That is the direct answer to "timing between
plugins". `scan_filter` and `realistic_chain_bench` use a filter that reads the whole
payload, to show the size-dependent work a real inspecting plugin pays.

Example: an empty chain runs in about 7 ns, and each added native filter costs roughly
1.25 ns (1 filter 8.4 ns, 2 → 9.7 ns, 4 → 12 ns, 8 → 17 ns). A payload-scanning filter
runs at about 6.4 GB/s.

## Layer B: frame codec

`cargo bench -p infrarust_protocol`

Isolates the transport codec the filter chain sits between: VarInt framing, zlib, AES,
and the full `PacketEncoder`/`PacketDecoder` paths, swept across payload sizes.

A few highlights from one run. VarInt decode is about 9 ns, encode about 14 ns. Decode is
near-free and size-independent (~46 ns), because payloads are zero-copy `Bytes` slices.
Uncompressed encode is a memcpy: ~70 ns at 32 B, ~6 µs at 16 KB.

Compression is the cost that matters. With the pure-Rust `flate2`/miniz backend, a 512 B
packet jumps from ~75 ns uncompressed to roughly 120 µs once it crosses the compression
threshold, because each packet rebuilds a fresh deflate stream.

The shipped `infrarust` binary defaults to the `libdeflater` backend (libdeflate, C) for a
~2-3x win. Build with `--no-default-features` to fall back to the pure-Rust flate2 backend.
The Layer B microbench (`cargo bench -p infrarust_protocol`) uses flate2 by default; add
`--features libdeflater` to reproduce the shipped backend.

The `*_compressed` minus `*_uncompressed` deltas attribute the zlib cost; the AES benches
attribute the online-mode encryption cost. AES-CFB8 is asymmetric: decryption computes
its keystream in batches (about 360 MB/s on the reference machine), while encryption is
sequential by construction, one full AES latency per byte (about 58 MB/s).

## Layer C: full intercepted CPU pipeline

`cargo bench -p infrarust-core --bench intercepted_pipeline`

Composes the real per-packet work the proxy does: decode, frame to `RawPacket`,
`CodecFilterChain::process`, `RawPacket` back to frame, encode, with no event-bus
listeners. This is the literal "ns per packet through intercepted mode" at the CPU level.
The socket I/O on top is measured end-to-end by Layer D.

Example with one inspecting plugin and no compression: about 106 ns at 32 B, 200 ns at
512 B, and 3.2 µs at 16 KB. Turn compression on and a re-encoded 512 B packet costs about
27 µs with the flate2 backend (6.3 µs with libdeflater), the same compression cliff
Layer B isolates, now in context.

The `forward_*` variants measure the filter-less forward path, where raw pass-through
applies: an unmodified compressed 512 B packet crosses the pipeline in about 120 ns and a
16 KiB one in about 1 µs (libdeflater), because the re-compression is skipped entirely
and only the inflate cost remains. The `empty_*`/`scan_*`/`realistic_*` numbers above are
what packets pay when something did modify them.

Read the deltas: `*_compressed` minus `*_uncompressed` is the zlib cost in context, and
`scan_*` or `realistic_*` minus `empty_*` is the codec-plugin cost in context.

## Layer D: end-to-end load

`tools/mc-bench`

`mc-bench` drives real Minecraft Play traffic through a running proxy and records RTT with
HdrHistogram under sustained, open-loop load (constant arrival rate, to avoid
[coordinated omission](https://www.youtube.com/watch?v=lJ8ydIuPFeU)). It speaks the
protocol through Infrarust's own `infrarust_protocol` crate and targets a pre-1.20.2
protocol, which skips the config phase. See
[`tools/mc-bench/README.md`](https://github.com/Shadowner/Infrarust/blob/main/tools/mc-bench/README.md)
for the full flags.

```bash
# 1. mock backend
cargo run -p infrarust-mc-bench --release -- serve-backend --port 25566

# 2a. baseline: client to backend directly
cargo run -p infrarust-mc-bench --release -- load \
  --host 127.0.0.1 --port 25566 --concurrency 500 --duration 120 --warmup 60

# 2b. through Infrarust (offline route to 127.0.0.1:25566): point --port at the proxy
cargo run -p infrarust-mc-bench --release -- load \
  --host 127.0.0.1 --port 25565 --server-address mc.example.com \
  --concurrency 500 --duration 120 --warmup 60
```

The proxy-added latency is the delta between (2b) and (2a). The tool reports connection
counts, throughput (echoes/sec), and mean plus p50/p90/p99/p99.9/max latency in
microseconds. To measure the compressed path, start the backend with `--compression 256`
and pass `--payload-size 512` to `load`: the pings then cross the compression threshold
in both directions. `tools/stress-test` is the tool for SLP/status-flood and malformed-packet
resilience; it never enters Play state.

## Layer E: live per-filter timing

`cargo run -p infrarust --features infrarust-core/bench-timing`

A feature flag, off by default and compiled out of normal builds, that times each filter
and the per-packet total inside a running proxy. It emits on the `infrarust::bench_timing`
tracing target:

```bash
cargo run -p infrarust --features infrarust-core/bench-timing
# then enable the target, e.g.:  RUST_LOG=infrarust::bench_timing=trace
```

`codec_filter` events carry the filter id and `ns`; `codec_packet` events carry the packet
id, length, and total `ns`. Use this to attribute latency to a specific plugin under
production-like conditions, or route it through the OpenTelemetry layer when the
`telemetry` feature is on.

## WASM codec filters: cost and isolation

`crates/infrarust-loader-wasm/benches/codec_exec/` loads real WASM plugins (the `fault-lab`, `codec-modify` and `scripted` test fixtures) through the loader and drives the real `CodecFilterChain`. It measures what a WASM codec filter costs on the healthy path and what a filter that runs away costs everyone else.

| Scenario | What it measures |
|----------|------------------|
| `hot`, `rr` | ns per 512 B packet on one connection, and round robin over 1000 connections, with and without an `await` between packets |
| `create` | µs to build and close both sides of a connection, 1000 concurrent creations, RSS per live connection |
| `sizes` | one call on 16 KiB to 2 MiB packets |
| `load` | 1000 connections fed 20 packets/s each through loopback sockets: latency and CPU per packet |
| `idle` | CPU of an idle proxy, epoch thread included |
| `isolation` | a healthy plugin's command and other connections' packets (fed through sockets) while attackers from one address make a filter spin, in `filter`, in `create` (`STUDY_ATTACK=create`) or in bursts of prepared connections (`STUDY_ATTACK=burst`) |

```bash
cargo bench -p infrarust-loader-wasm --features wasm --bench codec_exec --no-run
H=crates/infrarust-loader-wasm/benches/codec_exec
python3 $H/run.py isolation --profile sparse --configs default --runs 3
PART=isolation CONFIGS=default $H/study.sh <out-dir>
STUDY_BINARY_BEFORE=<older codec_exec binary> CONFIGS=before,default $H/study.sh <out-dir>
```

`run.py` runs each configuration in turn, repeats the series, and prints the median with the min and max. The `before` configuration runs another build of the bench, for a before/after comparison on the same host. Profiles: `sparse` (default worker count, 20 victim connections at 10 packets/s, 10 attackers each triggered by its own socket once a second), `sparse-w2pinned` (the same on 2 worker threads pinned to 2 cores with `taskset`), `w2pinned`, `w2` and `wdefault` (50 victims at 100 packets/s, attackers in a loop). `$H/soak.sh` replays the `faulty-codec` scenario of `tests/soak/` on the real binary.

Example, 3 runs on a 16-thread Ryzen 7 3700X shared with other builds (load average 7 to 27), before and after the 5 ms counted codec budget and the per-address quarantine:

| Measure | Before (800 ms budget) | After (5 ms, quarantine) |
|---------|------------------------|--------------------------|
| Other connections' packets, p99, 16 workers, sparse | 770 ms | 0.20 ms |
| Healthy plugin command, p99, same run | 768 ms | 4.0 ms |
| Other connections' packets, p99, 2 workers on 2 cores, sparse | 8.7 s | 0.27 ms |
| Other connections' packets, p99, 2 workers on 2 cores, dense | 3.4 s | 4.5 ms |
| Filter spinning in `create`: other packets p99 / attacker connection setup | 11.2 s / 1.66 s | 3.0 ms / 0.18 ms |
| Bursts of 25 prepared connections, 2 workers on 2 cores: other packets p99 | 9.6 s | 5.6 ms |
| CPU burnt by the attackers, sparse | 7.3 cores | 0.04 cores |
| 512 B packet with an `await` between packets, one connection / round robin | 1.14 µs / 2.92 µs | 1.06 µs / 1.60 µs |
| Connection create and close, sequential | 101 µs | 90 µs |
| RSS per live connection | 76 KiB | 76 KiB |
| Idle CPU (1 ms epoch tick) | 0.0% of a core | 0.4% of a core |
| Real binary, `tests/soak/run.sh --scenario faulty-codec` (16 workers, 10 attacking bots): mc-bench packet RTT p50 / p99 | 215 ms / 747 ms | 0.28 ms / 0.5 ms (0.28 / 0.6 without attackers) |
| Same run: chat echo p99 / proxy CPU | 805 ms / 205% | 30 ms / 2% (27 ms / 1% without attackers) |

The per-packet cost of a healthy filter does not change. What changes is that a filter call can no longer hold a worker thread, and with it the network driver of a lightly loaded proxy, for more than a few milliseconds.

## WASM plugins: what the contract costs

`crates/infrarust-loader-wasm/benches/wasm_contract/` loads the `perf-probe` test fixture through the public loader and fires real events on the real event bus. Each measurement runs 3 times in fresh runtimes and the tables give the median and the spread.

| Scenario | What it measures |
|----------|------------------|
| `events` | p50 and p99 of one `fire` with one WASM listener against one native listener: `PlayerClientBrandEvent` (cheap), `ProxyPingEvent` untouched and with its max players changed, `GameProfileRequestEvent` untouched and renamed; then events per second through one plugin with 16 tasks firing |
| `hostcalls` | ns per host call made from a WASM handler (a named event loops N calls, minus the 1-call event): `players.list` with 0, 100 and 1,000 online players carrying a textures property, `players.count`, `config.get`, a `trace!` the proxy does not log, an `info!` it logs; native equivalents next to them |
| `codec` | ns per 512 B packet through the `perf-probe` codec filter, with and without a `trace!` the proxy does not log |

```bash
cargo bench -p infrarust-loader-wasm --features wasm --bench wasm_contract -- events hostcalls codec
CONTRACT_RUNS=5 CONTRACT_SCALE=0.2 cargo bench -p infrarust-loader-wasm --features wasm --bench wasm_contract -- events
```

A tracing subscriber that logs `info` and above is installed for the whole run, as in a proxy with the default log level. `codec_boundary.rs` also measures a 512 B packet served round robin over 1,000 open connections next to one hot connection, both on demand and with `instance_pool = 4096`.

Example, 3 runs on a 16-thread Ryzen 7 3700X (load average 3), before and after the heavy ping fields stayed on the host, `players.list` answered light records, the SDK tested the log level before formatting and the capability gates were resolved at compile time:

| Measure | Before | After |
|---------|--------|-------|
| `PlayerClientBrandEvent`, one WASM listener, p50 / p99 (native 0.13 µs) | 2.77 µs / 5.2 µs | 2.84 µs / 6.0 µs |
| `ProxyPingEvent`, handler reads the virtual host, p50 / p99 (native 0.16 µs) | 7.96 µs / 11.2 µs | 3.75 µs / 5.5 µs |
| `ProxyPingEvent`, handler changes the max players, p50 / p99 | 12.7 µs / 26.0 µs | 3.06 µs / 5.8 µs |
| `GameProfileRequestEvent` untouched / renamed, p50 | 4.59 µs / 6.08 µs | 4.60 µs / 6.30 µs |
| `ProxyPingEvent` through one plugin, 16 tasks firing (native 6.7 to 9.5 M/s) | 109,000/s | 344,000/s |
| `PlayerClientBrandEvent` through one plugin, 16 tasks firing | 415,000/s | 416,000/s |
| `players.list` with 0 / 100 / 1,000 players online (native `get_all_players` at 1,000: 10 µs) | 345 ns / 136 µs / 1.25 ms | 205 ns / 19 µs / 0.18 ms |
| `players.count` / `config.get` host call | 263 ns / 367 ns | 118 ns / 290 ns |
| `trace!` at a level the proxy does not log / `info!` it logs | 243 ns / 335 ns | 4 ns / 355 ns |
| Capability check of a host call (`CapabilitySet::has`) | 12.7 ns | 0.5 ns |
| Codec filter, 512 B pass / with a `trace!` the proxy does not log | 274 ns / 489 ns | 283 ns / 297 ns |
| `codec_boundary`, 512 B pass on one hot connection / round robin over 1,000 connections | 286 ns / 541 ns | 279 ns / 532 ns |
| The same with `instance_pool = 4096` | 291 ns / 631 ns | 282 ns / 544 ns |

The round robin lines are the figure to plan with once many players are connected: each packet then reaches an instance whose memory the CPU caches no longer hold, which costs about twice the hot figure. Before and after, the codec rows measure the same code; their difference is noise.

## Quick reference

```bash
cargo bench -p infrarust-core      --bench codec_chain          # Layer A
cargo bench -p infrarust_protocol  --bench frame_codec          # Layer B
cargo bench -p infrarust-core      --bench intercepted_pipeline # Layer C
# Layer D: see tools/mc-bench/README.md
# Layer E: run the proxy with --features infrarust-core/bench-timing
cargo bench -p infrarust-loader-wasm --features wasm --bench codec_exec -- isolation  # WASM codec isolation
cargo bench -p infrarust-loader-wasm --features wasm --bench wasm_contract            # WASM contract costs
cargo bench -p infrarust-loader-wasm --features wasm --bench codec_boundary           # WASM codec boundary
```

Divan takes `--sample-count` and `--sample-size` for quicker runs, and filters by name,
for example `cargo bench -p infrarust-core --bench codec_chain -- filter_overhead`.

## Notes and follow-ups

The native-vs-WASM codec boundary is measured separately by
`infrarust-loader-wasm/benches/codec_boundary.rs`, which needs the `wasm32-wasip2` target
and built fixtures. Its per-packet figures come from one hot instance, which the CPU caches
hold; its round robin line over 1,000 open connections is the figure to plan with for a proxy
with many players (see the table above). Migrating it to divan is a mechanical follow-up gated on that fixture
build.

For deterministic CI regression gates, `iai-callgrind` (instruction counts, immune to CI
noise) layers cleanly onto Layers A to C, though it is not wired into CI yet. For CPU hotspot
analysis under load, profile the proxy with `samply` or `cargo flamegraph` while Layer D
applies load.
