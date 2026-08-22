//! Full intercepted-mode per-packet CPU pipeline (Layer C).
//!
//! Composes the real per-packet work the proxy does in `handle_client_to_backend`
//! (Play state, no event-bus listeners): `PacketDecoder::try_next_frame` →
//! `frame → RawPacket` → `CodecFilterChain::process` → `RawPacket → frame` →
//! `PacketEncoder::append_raw`. This is the literal "how many ns does a packet
//! take to go through intercepted mode" at the CPU level — the socket I/O on top
//! is measured end-to-end by the `mc-bench` load tool (Layer D).
//!
//! The deltas isolate each cost:
//! - `*_compressed` − `*_uncompressed` = zlib (de)compression cost in context.
//! - `scan`/`realistic` − `empty` = codec-plugin cost in context.
//!
//! Run with: `cargo bench -p infrarust-core --bench intercepted_pipeline`

use divan::counter::BytesCount;
use divan::{Bencher, black_box};

use infrarust_api::types::RawPacket;
use infrarust_core::filter::codec_chain::CodecFilterChain;
use infrarust_protocol::packets::play::chat::{SChatCommand, SChatMessage};
use infrarust_protocol::packets::play::chat_session::SChatSessionUpdate;
use infrarust_protocol::packets::play::tab_complete::STabCompleteRequest;
use infrarust_protocol::registry::build_default_registry;
use infrarust_protocol::version::ProtocolVersion;
use infrarust_protocol::{PacketDecoder, PacketEncoder};

#[path = "shared/filters.rs"]
mod filters;
use filters::{PROTOCOL, SIZES, pass_chain, payload_vec, realistic_chain, scan_chain};

fn main() {
    divan::main();
}

const THRESHOLD: i32 = 256;

/// On-wire bytes of one frame, as the proxy receives it from a socket.
fn frame_bytes(size: usize, compression: Option<i32>) -> Vec<u8> {
    let mut enc = PacketEncoder::new();
    if let Some(t) = compression {
        enc.set_compression(t);
    }
    enc.append_raw(0x10, &payload_vec(size)).unwrap();
    enc.take().to_vec()
}

/// Times one full decode → filter-chain → encode round trip per iteration,
/// re-queuing a fresh frame each time (decoder/encoder reused across iterations
/// exactly as the per-connection bridges reuse them).
fn run(bencher: Bencher, size: usize, compression: Option<i32>, mut chain: CodecFilterChain) {
    let bytes = frame_bytes(size, compression);
    let mut decoder = PacketDecoder::new();
    let mut encoder = PacketEncoder::new();
    if let Some(t) = compression {
        decoder.set_compression(t);
        encoder.set_compression(t);
    }
    bencher.counter(BytesCount::new(size)).bench_local(|| {
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        let mut raw = RawPacket::new(frame.id, frame.payload);
        black_box(chain.process(&mut raw));
        encoder.append_raw(raw.packet_id, &raw.data).unwrap();
        black_box(encoder.take())
    });
}

// --- Empty chain: transport-only intercepted cost (decode + encode) ----------

#[divan::bench(args = SIZES)]
fn empty_uncompressed(bencher: Bencher, size: usize) {
    run(bencher, size, None, pass_chain(0));
}

#[divan::bench(args = SIZES)]
fn empty_compressed(bencher: Bencher, size: usize) {
    run(bencher, size, Some(THRESHOLD), pass_chain(0));
}

// --- One inspecting plugin ---------------------------------------------------

#[divan::bench(args = SIZES)]
fn scan_uncompressed(bencher: Bencher, size: usize) {
    run(bencher, size, None, scan_chain());
}

#[divan::bench(args = SIZES)]
fn scan_compressed(bencher: Bencher, size: usize) {
    run(bencher, size, Some(THRESHOLD), scan_chain());
}

// --- Realistic small plugin stack (scan + 2 pass) ----------------------------

#[divan::bench(args = SIZES)]
fn realistic_uncompressed(bencher: Bencher, size: usize) {
    run(bencher, size, None, realistic_chain());
}

fn run_forward(bencher: Bencher, size: usize, compression: Option<i32>) {
    let bytes = frame_bytes(size, compression);
    let mut decoder = PacketDecoder::new();
    let mut encoder = PacketEncoder::new();
    if let Some(t) = compression {
        decoder.set_compression(t);
        encoder.set_compression(t);
    }
    bencher.counter(BytesCount::new(size)).bench_local(|| {
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        encoder.append_frame(&frame).unwrap();
        black_box(encoder.take())
    });
}

#[divan::bench(args = SIZES)]
fn forward_uncompressed(bencher: Bencher, size: usize) {
    run_forward(bencher, size, None);
}

#[divan::bench(args = SIZES)]
fn forward_compressed(bencher: Bencher, size: usize) {
    run_forward(bencher, size, Some(THRESHOLD));
}

fn hot_ids(registry: &infrarust_protocol::PacketRegistry) -> [Option<i32>; 4] {
    let version = ProtocolVersion(PROTOCOL);
    [
        registry.get_packet_id::<SChatSessionUpdate>(version),
        registry.get_packet_id::<STabCompleteRequest>(version),
        registry.get_packet_id::<SChatCommand>(version),
        registry.get_packet_id::<SChatMessage>(version),
    ]
}

#[divan::bench]
fn registry_hot_lookups(bencher: Bencher) {
    let registry = build_default_registry();
    bencher.bench_local(|| black_box(hot_ids(black_box(&registry))));
}

#[divan::bench]
fn registry_hot_lookups_cached(bencher: Bencher) {
    let registry = build_default_registry();
    let ids = hot_ids(&registry);
    bencher.bench_local(|| {
        let id = Some(black_box(0x10_i32));
        black_box(black_box(&ids).contains(&id))
    });
}
