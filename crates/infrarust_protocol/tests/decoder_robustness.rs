#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::LazyLock;

use bytes::Bytes;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::registry::{PacketRegistry, build_default_registry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use proptest::prelude::*;

type DecoderKey = (ConnectionState, Direction, ProtocolVersion, i32);

static REGISTRY: LazyLock<PacketRegistry> = LazyLock::new(build_default_registry);

static DECODERS: LazyLock<Vec<DecoderKey>> = LazyLock::new(|| {
    let states = [
        ConnectionState::Handshake,
        ConnectionState::Status,
        ConnectionState::Login,
        ConnectionState::Config,
        ConnectionState::Play,
    ];
    let mut keys = Vec::new();
    for state in states {
        for direction in [Direction::Serverbound, Direction::Clientbound] {
            for &version in ProtocolVersion::SUPPORTED {
                for id in 0..=0xFF {
                    if REGISTRY.has_decoder(state, direction, version, id) {
                        keys.push((state, direction, version, id));
                    }
                }
            }
        }
    }
    keys
});

fn decode(key: DecoderKey, bytes: &[u8]) {
    let (state, direction, version, id) = key;
    let frame = PacketFrame::new(id, Bytes::copy_from_slice(bytes));
    let _ = REGISTRY.decode_frame(&frame, state, direction, version);
}

#[test]
fn every_decoder_survives_degenerate_payloads() {
    assert!(!DECODERS.is_empty());
    let patterns: [&[u8]; 6] = [
        &[],
        &[0x00; 64],
        &[0xFF; 64],
        &[0x80; 64],
        &[0xFF, 0xFF, 0xFF, 0xFF, 0x07],
        &[0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x01],
    ];
    for &key in DECODERS.iter() {
        for pattern in patterns {
            for len in 0..=pattern.len() {
                decode(key, &pattern[..len]);
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn every_decoder_survives_arbitrary_bytes(
        index in any::<prop::sample::Index>(),
        bytes in prop::collection::vec(any::<u8>(), 0..512),
    ) {
        decode(*index.get(&DECODERS), &bytes);
    }

    #[test]
    fn every_decoder_survives_small_arbitrary_bytes(
        index in any::<prop::sample::Index>(),
        bytes in prop::collection::vec(prop_oneof![Just(0u8), Just(1), Just(0xFF), any::<u8>()], 0..24),
    ) {
        decode(*index.get(&DECODERS), &bytes);
    }
}
