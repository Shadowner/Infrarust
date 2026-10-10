use std::hint::black_box;
use std::time::Instant;

use bytes::Bytes;
use infrarust_api::filter::{
    CodecFilterFactory, CodecFilterInstance, CodecSessionInit, CodecVerdict, FilterMetadata,
    FrameOutput,
};
use infrarust_api::types::{ProtocolVersion, RawPacket};
use infrarust_core::filter::codec_chain::{CodecFilterChain, build_codec_chains};
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;

use crate::probe;
use crate::report::{Table, per_iter_ns, runtime, scaled};

const PASS_PACKET: i32 = 0x10;
const TRACE_PACKET: i32 = 0x20;

struct NativePassthroughFactory;

impl CodecFilterFactory for NativePassthroughFactory {
    fn metadata(&self) -> FilterMetadata {
        FilterMetadata::new("native-passthrough")
    }

    fn create(&self, _init: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
        Box::new(NativePassthrough)
    }
}

struct NativePassthrough;

impl CodecFilterInstance for NativePassthrough {
    fn filter(&mut self, _packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
        CodecVerdict::Pass
    }
}

fn client_chain(registry: &CodecFilterRegistryImpl) -> CodecFilterChain {
    let (client, _server) = build_codec_chains(
        registry,
        ProtocolVersion::new(767),
        1,
        "127.0.0.1:1".parse().unwrap(),
        None,
    );
    client
}

fn ns_per_packet(chain: &mut CodecFilterChain, id: i32, iterations: usize) -> f64 {
    let mut packet = RawPacket::new(id, Bytes::from(vec![0xABu8; 512]));
    for _ in 0..iterations / 10 {
        black_box(chain.process(black_box(&mut packet)));
    }
    let started = Instant::now();
    for _ in 0..iterations {
        black_box(chain.process(black_box(&mut packet)));
    }
    per_iter_ns(started, iterations)
}

pub(crate) fn run(runs: usize) {
    println!("## Codec filter: a log call at a level the proxy does not log\n");
    let iterations = scaled(200_000);
    let mut table = Table::new(
        format!("perf-probe codec filter, 512 B packets, {iterations} packets per point"),
        "ns per packet",
    );
    for _ in 0..runs {
        let native_registry = CodecFilterRegistryImpl::new();
        native_registry
            .register_builtin(Box::new(NativePassthroughFactory))
            .unwrap();
        let mut native = client_chain(&native_registry);
        let rt = runtime(2);
        let probe = rt.block_on(probe::load_default());
        let mut chain = client_chain(&probe.env.codec_registry);
        table.record(
            "native passthrough",
            ns_per_packet(&mut native, PASS_PACKET, iterations),
        );
        table.record(
            "WASM filter, pass",
            ns_per_packet(&mut chain, PASS_PACKET, iterations),
        );
        table.record(
            "WASM filter, pass + trace! at a level the proxy does not log",
            ns_per_packet(&mut chain, TRACE_PACKET, iterations),
        );
        chain.close();
        drop(chain);
        drop(probe);
        rt.shutdown_background();
    }
    table.print();
}
