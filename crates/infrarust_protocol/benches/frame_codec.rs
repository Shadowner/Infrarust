use divan::counter::BytesCount;
use divan::{Bencher, black_box};

use infrarust_protocol::codec::VarInt;
use infrarust_protocol::crypto::{DecryptCipher, EncryptCipher};
use infrarust_protocol::{PacketDecoder, PacketEncoder};

fn main() {
    divan::main();
}

const SIZES: &[usize] = &[32, 128, 512, 2048, 4096, 16384];
const THRESHOLD: i32 = 256;

fn payload(size: usize) -> Vec<u8> {
    let mut state: u32 = 0x9E37_79B9 ^ (size as u32);
    (0..size)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

fn encoded_frame(id: i32, payload: &[u8], compression: Option<i32>) -> Vec<u8> {
    let mut enc = PacketEncoder::new();
    if let Some(t) = compression {
        enc.set_compression(t);
    }
    enc.append_raw(id, payload).unwrap();
    enc.take().to_vec()
}

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
