//! Shared synthetic codec filters + helpers for the `codec_chain` and
//! `intercepted_pipeline` benches. Included via `#[path]` (not a bench target
//! itself — cargo only auto-discovers `benches/<name>.rs`).
#![allow(dead_code)] // each including bench uses a subset

use bytes::Bytes;

use infrarust_api::filter::{
    CodecFilterFactory, CodecFilterInstance, CodecFilterRegistry, CodecSessionInit, CodecVerdict,
    FilterMetadata, FrameOutput,
};
use infrarust_api::types::{ProtocolVersion, RawPacket};
use infrarust_core::filter::codec_chain::{CodecFilterChain, build_codec_chains};
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;

/// Representative Minecraft payload sizes: keepalive/movement (32–128B), small
/// play packets (512B–2K), chunk-batch territory (4K–16K).
pub const SIZES: &[usize] = &[32, 128, 512, 2048, 4096, 16384];
pub const COUNTS: &[usize] = &[0, 1, 2, 4, 8];
pub const PROTOCOL: i32 = 767; // 1.21.x

/// Deterministic, hard-to-compress payload so any zlib cost downstream is
/// realistic rather than the degenerate near-zero of an all-same-byte buffer.
pub fn payload_vec(size: usize) -> Vec<u8> {
    let mut state: u32 = 0x9E37_79B9 ^ (size as u32);
    (0..size)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

pub fn payload(size: usize) -> Bytes {
    Bytes::from(payload_vec(size))
}

/// A do-nothing passthrough filter: isolates pure per-filter dispatch overhead.
struct PassInstance;
impl CodecFilterInstance for PassInstance {
    fn filter(&mut self, _packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
        CodecVerdict::Pass
    }
}

pub struct PassFactory {
    pub id: String,
}
impl CodecFilterFactory for PassFactory {
    fn metadata(&self) -> FilterMetadata {
        FilterMetadata::new(self.id.clone())
    }
    fn create(&self, _init: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
        Box::new(PassInstance)
    }
}

/// A filter that reads the whole payload (e.g. a content inspector): models the
/// payload-dependent work a real plugin does on top of the dispatch overhead.
struct ScanInstance {
    acc: u64,
}
impl CodecFilterInstance for ScanInstance {
    fn filter(&mut self, packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
        let sum = packet
            .data
            .iter()
            .fold(0u64, |a, &b| a.wrapping_add(u64::from(b)));
        self.acc ^= sum;
        CodecVerdict::Pass
    }
}

pub struct ScanFactory {
    pub id: String,
}
impl CodecFilterFactory for ScanFactory {
    fn metadata(&self) -> FilterMetadata {
        FilterMetadata::new(self.id.clone())
    }
    fn create(&self, _init: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
        Box::new(ScanInstance { acc: 0 })
    }
}

/// Builds the client-side chain from a registry populated by `populate`.
pub fn build_chain(populate: impl FnOnce(&CodecFilterRegistryImpl)) -> CodecFilterChain {
    let registry = CodecFilterRegistryImpl::new();
    populate(&registry);
    let (client, _server) = build_codec_chains(
        &registry,
        ProtocolVersion::new(PROTOCOL),
        1,
        "127.0.0.1:0".parse().unwrap(),
        None,
    );
    client
}

/// Chain with `n` passthrough filters.
pub fn pass_chain(n: usize) -> CodecFilterChain {
    build_chain(|reg| {
        for i in 0..n {
            reg.register(Box::new(PassFactory {
                id: format!("pass-{i}"),
            }));
        }
    })
}

/// Chain with a single payload-scanning filter.
pub fn scan_chain() -> CodecFilterChain {
    build_chain(|reg| {
        reg.register(Box::new(ScanFactory {
            id: "scan".to_string(),
        }));
    })
}

/// A representative small plugin stack: one scanner + two passthroughs.
pub fn realistic_chain() -> CodecFilterChain {
    build_chain(|reg| {
        reg.register(Box::new(ScanFactory {
            id: "scan".to_string(),
        }));
        reg.register(Box::new(PassFactory {
            id: "pass-a".to_string(),
        }));
        reg.register(Box::new(PassFactory {
            id: "pass-b".to_string(),
        }));
    })
}
