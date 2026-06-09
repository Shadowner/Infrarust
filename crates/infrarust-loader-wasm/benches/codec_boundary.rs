#[cfg(all(feature = "wasm", wasm_fixtures_available))]
#[path = "../tests/mock_services/mod.rs"]
mod mock_services;

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
fn main() {
    bench::run();
}

#[cfg(not(all(feature = "wasm", wasm_fixtures_available)))]
fn main() {
    eprintln!(
        "codec_boundary: requires `--features wasm` and built wasm fixtures (skipped). \
         Install the wasm32-wasip2 target and rebuild."
    );
}

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
mod bench {
    use std::collections::HashMap;
    use std::hint::black_box;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Instant;

    use bytes::Bytes;
    use infrarust_api::filter::{
        CodecContext, CodecFilterFactory, CodecFilterInstance, CodecFilterRegistry,
        CodecSessionInit, CodecVerdict, FilterMetadata, FrameOutput,
    };
    use infrarust_api::loader::{PluginContextFactory, PluginLoader};
    use infrarust_api::types::{ProtocolVersion, RawPacket};
    use infrarust_config::ProxyConfig;
    use infrarust_core::event_bus::EventBusImpl;
    use infrarust_core::filter::codec_chain::{CodecFilterChain, build_codec_chains};
    use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;
    use infrarust_core::filter::transport_registry::TransportFilterRegistryImpl;
    use infrarust_core::plugin::manager::PluginServices;
    use infrarust_core::plugin::{PluginContextFactoryImpl, PluginPermissions, PluginRegistryImpl};
    use infrarust_core::routing::DomainRouter;
    use infrarust_core::services::command_manager::CommandManagerImpl;
    use infrarust_core::services::scheduler::SchedulerImpl;
    use infrarust_core::services::server_manager_bridge::NoopServerManager;
    use infrarust_loader_wasm::{WasmPluginLoader, build_engine};

    use super::mock_services::{MockBanService, MockConfigService, MockPlayerRegistry};

    const ITERS: u64 = 200_000;
    const WARMUP: u64 = 20_000;

    const SIZES: &[usize] = &[32, 512, 4096, 16384];

    /// A native passthrough filter — the baseline the WASM path is measured against.
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
        fn filter(
            &mut self,
            _ctx: &CodecContext,
            _packet: &mut RawPacket,
            _output: &mut FrameOutput,
        ) -> CodecVerdict {
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

    /// Times `chain.process` over a fixed, unchanged packet of `size` bytes (id
    /// `0x10` passes through both filters untouched), returning ns/packet.
    fn ns_per_packet(chain: &mut CodecFilterChain, size: usize) -> f64 {
        let mut packet = RawPacket::new(0x10, Bytes::from(vec![0xABu8; size]));
        for _ in 0..WARMUP {
            let _ = black_box(chain.process(black_box(&mut packet)));
        }
        let start = Instant::now();
        for _ in 0..ITERS {
            let _ = black_box(chain.process(black_box(&mut packet)));
        }
        start.elapsed().as_nanos() as f64 / ITERS as f64
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("INFRARUST_WASM_FIXTURE_DIR"))
            .join(format!("fixture_{}.wasm", name.replace('-', "_")))
    }

    /// Loads `codec-modify`, enables it, and returns the registry it registered its
    /// filter into, plus the keep-alives that must outlive the chains.
    async fn load_wasm(
        plugins_dir: PathBuf,
    ) -> (
        Arc<CodecFilterRegistryImpl>,
        Box<dyn infrarust_api::plugin::Plugin>,
        WasmPluginLoader,
    ) {
        let registry = Arc::new(CodecFilterRegistryImpl::new());
        let services = PluginServices {
            event_bus: Arc::new(EventBusImpl::new()),
            player_registry: Arc::new(MockPlayerRegistry),
            server_manager: Arc::new(NoopServerManager),
            ban_service: Arc::new(MockBanService),
            command_manager: Arc::new(CommandManagerImpl::new()),
            scheduler: Arc::new(SchedulerImpl::new()),
            config_service: Arc::new(MockConfigService),
            plugin_registry: Arc::new(PluginRegistryImpl::new()),
            codec_filter_registry: Arc::clone(&registry),
            transport_filter_registry: Arc::new(TransportFilterRegistryImpl::new()),
            domain_router: Arc::new(DomainRouter::new()),
            proxy_shutdown: tokio_util::sync::CancellationToken::new(),
            proxy_info: infrarust_api::services::proxy_info::ProxyInfo::default(),
            plugins_dir: plugins_dir.clone(),
        };
        let mut configs = HashMap::new();
        configs.insert(
            "codec-modify".to_string(),
            PluginPermissions {
                permissions: vec!["codec-filter".to_string()],
                trusted: false,
            },
        );
        let factory = PluginContextFactoryImpl::new(services, configs);

        let config: ProxyConfig = toml::from_str("").unwrap();
        let loader = WasmPluginLoader::new(build_engine(&config).unwrap());
        loader.discover(&plugins_dir).await.unwrap();
        let plugin = loader.load("codec-modify", &factory).await.unwrap();
        let ctx = factory.create_context("codec-modify");
        plugin.on_enable(ctx.as_ref()).await.unwrap();
        (registry, plugin, loader)
    }

    fn staged_plugins_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("infrarust-codec-bench-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(fixture_path("codec-modify"), dir.join("codec-modify.wasm")).unwrap();
        dir
    }

    pub fn run() {
        // Native baseline.
        let native_registry = CodecFilterRegistryImpl::new();
        native_registry.register(Box::new(NativePassthroughFactory));
        let mut native_chain = client_chain(&native_registry);

        // WASM synchronous filter (production path).
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let plugins_dir = staged_plugins_dir();
        let (wasm_registry, _plugin, _loader) = rt.block_on(load_wasm(plugins_dir));
        let mut wasm_chain = client_chain(&wasm_registry);

        println!("\ncodec boundary benchmark — {ITERS} iterations/measurement, id 0x10 (pass-through, zero-copy return)");
        println!(
            "  {:>7}  {:>10}  {:>10}  {:>12}",
            "size", "native", "wasm", "boundary"
        );
        let mut boundary_32 = f64::NAN;
        for &size in SIZES {
            let native_ns = ns_per_packet(&mut native_chain, size);
            let wasm_ns = ns_per_packet(&mut wasm_chain, size);
            let boundary = wasm_ns - native_ns;
            if size == 32 {
                boundary_32 = boundary;
            }
            println!(
                "  {size:>5}B  {native_ns:>8.1}ns  {wasm_ns:>8.1}ns  {boundary:>10.1}ns",
            );
        }
        println!(
            "\n  fixed call overhead ≈ {boundary_32:.1} ns/packet (32B) — target 100–300 → {}\n",
            if boundary_32 <= 300.0 { "MET" } else { "over (see report)" }
        );
    }
}
