//! Frame-codec microbenchmarks (Layer B).
//!
//! Isolates the transport codec that the proxy's filter chain sits between:
//! VarInt framing, zlib (de)compression, AES-128-CFB8, and the full
//! `PacketEncoder`/`PacketDecoder` paths. Each is swept across representative
//! Minecraft payload sizes so we can attribute a packet's cost to framing vs
//! zlib vs crypto vs filtering (the latter measured in `infrarust-core`).
//!
//! Run with: `cargo bench -p infrarust_protocol`

use divan::counter::BytesCount;
use divan::{Bencher, black_box};

use infrarust_protocol::codec::VarInt;
use infrarust_protocol::crypto::{DecryptCipher, EncryptCipher};
use infrarust_protocol::{PacketDecoder, PacketEncoder};

fn main() {
    divan::main();
}

/// Payload sizes: keepalive/movement (32–128B), small play packets (512B–2K),
/// up to chunk-batch territory (4K–16K). Default compression threshold is 256,
/// so sizes < 256 stay uncompressed even with compression enabled.
const SIZES: &[usize] = &[32, 128, 512, 2048, 4096, 16384];
const THRESHOLD: i32 = 256;

/// Deterministic, hard-to-compress payload (LCG) so zlib cost is realistic
/// rather than the degenerate near-zero of an all-same-byte buffer.
fn payload(size: usize) -> Vec<u8> {
    let mut state: u32 = 0x9E37_79B9 ^ (size as u32);
    (0..size)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

/// Produces the on-wire bytes of one frame, as the decoder will receive them.
fn encoded_frame(id: i32, payload: &[u8], compression: Option<i32>) -> Vec<u8> {
    let mut enc = PacketEncoder::new();
    if let Some(t) = compression {
        enc.set_compression(t);
    }
    enc.append_raw(id, payload).unwrap();
    enc.take().to_vec()
}

// ---------------------------------------------------------------------------
// VarInt — the smallest unit; every frame has at least a length + id VarInt.
// ---------------------------------------------------------------------------

/// Values chosen to exercise 1- through 5-byte VarInt encodings.
const VARINT_VALUES: &[i32] = &[0, 127, 128, 16_383, 2_097_151, i32::MAX];

#[divan::bench(args = VARINT_VALUES)]
fn varint_encode(bencher: Bencher, value: i32) {
    bencher.bench(|| {
        let mut buf = Vec::with_capacity(5);
        VarInt(black_box(value)).encode(&mut buf).unwrap();
        black_box(buf)
    });
}

#[divan::bench(args = VARINT_VALUES)]
fn varint_decode(bencher: Bencher, value: i32) {
    let mut buf = Vec::with_capacity(5);
    VarInt(value).encode(&mut buf).unwrap();
    bencher.bench(|| {
        let mut cursor: &[u8] = black_box(&buf);
        black_box(VarInt::decode(&mut cursor).unwrap())
    });
}

/// Same as `varint_decode` but with trailing bytes after the VarInt (the
/// realistic framing case — the packet payload always follows the length/id
/// VarInt). The slack matters for any decoder sensitive to the available buffer
/// length, e.g. wide/SIMD loads that read past the VarInt's end.
#[divan::bench(args = VARINT_VALUES)]
fn varint_decode_padded(bencher: Bencher, value: i32) {
    let mut buf = Vec::new();
    VarInt(value).encode(&mut buf).unwrap();
    buf.resize(buf.len() + 16, 0);
    bencher.bench(|| {
        let mut cursor: &[u8] = black_box(&buf);
        black_box(VarInt::decode(&mut cursor).unwrap())
    });
}

// zlib cost is attributed via the `encode_compressed` − `encode_uncompressed`
// (and decode) deltas below; the `compression` module is crate-private, so it
// is benched through the public encoder/decoder rather than directly.

// ---------------------------------------------------------------------------
// AES-128-CFB8 — only paid on online-mode (client_only/full) connections.
// ---------------------------------------------------------------------------

#[divan::bench(args = SIZES)]
fn aes_encrypt(bencher: Bencher, size: usize) {
    let data = payload(size);
    bencher
        .counter(BytesCount::new(size))
        .with_inputs(|| (EncryptCipher::new(&[0x42; 16]), data.clone()))
        .bench_local_values(|(mut cipher, mut buf)| {
            cipher.encrypt(black_box(&mut buf));
            black_box(buf)
        });
}

#[divan::bench(args = SIZES)]
fn aes_decrypt(bencher: Bencher, size: usize) {
    let mut data = payload(size);
    EncryptCipher::new(&[0x42; 16]).encrypt(&mut data);
    bencher
        .counter(BytesCount::new(size))
        .with_inputs(|| (DecryptCipher::new(&[0x42; 16]), data.clone()))
        .bench_local_values(|(mut cipher, mut buf)| {
            cipher.decrypt(black_box(&mut buf));
            black_box(buf)
        });
}

// ---------------------------------------------------------------------------
// Full frame encode / decode — what the proxy actually calls per packet.
// ---------------------------------------------------------------------------

#[divan::bench(args = SIZES)]
fn encode_uncompressed(bencher: Bencher, size: usize) {
    let data = payload(size);
    bencher
        .counter(BytesCount::new(size))
        .with_inputs(PacketEncoder::new)
        .bench_local_values(|mut enc| {
            enc.append_raw(0x10, black_box(&data)).unwrap();
            black_box(enc.take())
        });
}

#[divan::bench(args = SIZES)]
fn encode_compressed(bencher: Bencher, size: usize) {
    let data = payload(size);
    bencher
        .counter(BytesCount::new(size))
        .with_inputs(|| {
            let mut enc = PacketEncoder::new();
            enc.set_compression(THRESHOLD);
            enc
        })
        .bench_local_values(|mut enc| {
            enc.append_raw(0x10, black_box(&data)).unwrap();
            black_box(enc.take())
        });
}

#[divan::bench(args = SIZES)]
fn decode_uncompressed(bencher: Bencher, size: usize) {
    let frame = encoded_frame(0x10, &payload(size), None);
    bencher
        .counter(BytesCount::new(size))
        .with_inputs(|| {
            let mut d = PacketDecoder::new();
            d.queue_bytes(&frame);
            d
        })
        .bench_local_values(|mut d| black_box(d.try_next_frame().unwrap()));
}

#[divan::bench(args = SIZES)]
fn decode_compressed(bencher: Bencher, size: usize) {
    let frame = encoded_frame(0x10, &payload(size), Some(THRESHOLD));
    bencher
        .counter(BytesCount::new(size))
        .with_inputs(|| {
            let mut d = PacketDecoder::new();
            d.set_compression(THRESHOLD);
            d.queue_bytes(&frame);
            d
        })
        .bench_local_values(|mut d| black_box(d.try_next_frame().unwrap()));
}
