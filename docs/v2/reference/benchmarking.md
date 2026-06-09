---
title: Benchmarking
description: How Infrarust's intercepted-mode packet path is benchmarked (per-packet latency, per-plugin overhead, and end-to-end load), with the commands to reproduce every number.
---

# Benchmarking

Infrarust's intercepted modes (`client_only`, `offline`, `full`) decode every packet,
run it through the codec filter chain, and re-encode it. This page covers how that hot
path is measured, so you can answer two questions with real numbers:

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
| [divan](https://github.com/nvzqz/divan) | sub-microsecond microbenchmarks (Layers A–C), `harness = false` |
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
attribute the online-mode encryption cost.

## Layer C: full intercepted CPU pipeline

`cargo bench -p infrarust-core --bench intercepted_pipeline`

Composes the real per-packet work the proxy does: decode, frame to `RawPacket`,
`CodecFilterChain::process`, `RawPacket` back to frame, encode, with no event-bus
listeners. This is the literal "ns per packet through intercepted mode" at the CPU level.
The socket I/O on top is measured end-to-end by Layer D.

Example with one inspecting plugin and no compression: about 106 ns at 32 B, 200 ns at
512 B, and 3.2 µs at 16 KB. Turn compression on and a 512 B packet costs about 27 µs, the
same compression cliff Layer B isolates, now in context.

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

The proxy-added latency is the delta between (2b) and (2a). The tool reports throughput
and p50/p90/p99/p99.9/max. `tools/stress-test` stays the tool for SLP/status-flood and
malformed-packet resilience; it never enters Play state.

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

## Quick reference

```bash
cargo bench -p infrarust-core      --bench codec_chain          # Layer A
cargo bench -p infrarust_protocol  --bench frame_codec          # Layer B
cargo bench -p infrarust-core      --bench intercepted_pipeline # Layer C
# Layer D: see tools/mc-bench/README.md
# Layer E: run the proxy with --features infrarust-core/bench-timing
```

Divan takes `--sample-count` and `--sample-size` for quicker runs, and filters by name,
for example `cargo bench -p infrarust-core --bench codec_chain -- filter_overhead`.

## Notes and follow-ups

The native-vs-WASM codec boundary is measured separately by
`infrarust-loader-wasm/benches/codec_boundary.rs`, which needs the `wasm32-wasip2` target
and built fixtures. Migrating it to divan is a mechanical follow-up gated on that fixture
build.

For deterministic CI regression gates, `iai-callgrind` (instruction counts, immune to CI
noise) layers cleanly onto Layers A–C, though it is not wired into CI yet. For CPU hotspot
analysis under load, profile the proxy with `samply` or `cargo flamegraph` while Layer D
applies load.
