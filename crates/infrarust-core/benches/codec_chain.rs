//! Codec filter chain microbenchmarks (Layer A) — "timing between plugins".
//!
//! `CodecFilterChain::process` is the sync per-packet call that every codec
//! plugin/filter runs through in intercepted mode (`session/proxy_loop.rs` →
//! `apply_codec_filter` → `chain.process`). This bench attributes cost two ways:
//!
//! - `filter_overhead`: marginal ns added by each *additional* passthrough
//!   filter in the chain (0→1→2→4→8) at a fixed payload — i.e. the framework's
//!   per-plugin dispatch overhead, independent of what the plugin does. The
//!   delta between successive counts is the deterministic answer to "timing
//!   between plugins".
//! - `scan_filter` / `realistic_chain`: payload-dependent work for a filter
//!   that actually inspects the packet bytes.
//!
//! All filters here are native (in-process). The native-vs-WASM boundary is
//! measured separately in `infrarust-loader-wasm/benches/codec_boundary.rs`.
//!
//! Run with: `cargo bench -p infrarust-core --bench codec_chain`

use divan::counter::{BytesCount, ItemsCount};
use divan::{Bencher, black_box};

use infrarust_api::types::RawPacket;

#[path = "shared/filters.rs"]
mod filters;
use filters::{COUNTS, SIZES, payload, pass_chain, realistic_chain, scan_chain};

fn main() {
    divan::main();
}

/// Per-filter dispatch overhead: the delta between successive counts is the cost
/// the plugin *framework* adds per plugin, independent of payload size.
#[divan::bench(args = COUNTS)]
fn filter_overhead(bencher: Bencher, count: usize) {
    let mut chain = pass_chain(count);
    let mut packet = RawPacket::new(0x10, payload(256));
    bencher
        .counter(ItemsCount::new(1usize))
        .bench_local(|| black_box(chain.process(black_box(&mut packet))));
}

/// A single payload-scanning filter across payload sizes: the size-dependent
/// cost a real inspecting plugin pays.
#[divan::bench(args = SIZES)]
fn scan_filter(bencher: Bencher, size: usize) {
    let mut chain = scan_chain();
    let mut packet = RawPacket::new(0x10, payload(size));
    bencher
        .counter(BytesCount::new(size))
        .bench_local(|| black_box(chain.process(black_box(&mut packet))));
}

/// A representative small plugin stack (one scanner + two passthroughs) across
/// sizes: the realistic per-packet chain cost on the proxy hot path.
#[divan::bench(args = SIZES)]
fn realistic_chain_bench(bencher: Bencher, size: usize) {
    let mut chain = realistic_chain();
    let mut packet = RawPacket::new(0x10, payload(size));
    bencher
        .counter(BytesCount::new(size))
        .bench_local(|| black_box(chain.process(black_box(&mut packet))));
}

/// Empty-chain baseline: `process` early-returns `Pass` when no filters are
/// registered (the cost an intercepted connection pays with zero codec plugins).
#[divan::bench(args = SIZES)]
fn empty_chain(bencher: Bencher, size: usize) {
    let mut chain = pass_chain(0);
    let mut packet = RawPacket::new(0x10, payload(size));
    bencher
        .counter(ItemsCount::new(1usize))
        .bench_local(|| black_box(chain.process(black_box(&mut packet))));
}
