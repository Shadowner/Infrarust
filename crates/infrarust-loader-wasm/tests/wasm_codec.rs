//! Phase C end-to-end: a WASM-registered codec filter drives the native chain.
//!
//! Compiles to nothing unless the `wasm` feature is on AND the fixtures built
//! (`wasm_fixtures_available`), like `wasm_fixtures.rs`.
#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_api::plugin::Plugin;
use infrarust_api::types::{ProtocolVersion, RawPacket};
use infrarust_config::ProxyConfig;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::filter::codec_chain::{CodecFilterChain, FilterResult, build_codec_chains};
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;
use infrarust_core::filter::transport_registry::TransportFilterRegistryImpl;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::{PluginContextFactoryImpl, PluginPermissions, PluginRegistryImpl};
use infrarust_core::routing::DomainRouter;
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;
use infrarust_loader_wasm::{WasmPluginLoader, build_engine};

mod mock_services;
use mock_services::{
    MockBanService, MockConfigService, MockLoadBalancerService, MockPlayerRegistry,
};

const FIXTURE_DIR: &str = env!("INFRARUST_WASM_FIXTURE_DIR");

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(FIXTURE_DIR).join(format!("fixture_{}.wasm", name.replace('-', "_")))
}

fn fresh_loader() -> WasmPluginLoader {
    let config: ProxyConfig = toml::from_str("").expect("default proxy config");
    WasmPluginLoader::new(build_engine(&config).expect("build engine"))
}

fn stage(fixture: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    std::fs::copy(
        fixture_path(fixture),
        plugins_dir.join(format!("{fixture}.wasm")),
    )
    .unwrap();
    (tmp, plugins_dir)
}

fn codec_env(
    plugins_dir: PathBuf,
    plugin_id: &str,
    grant: bool,
) -> (PluginContextFactoryImpl, Arc<CodecFilterRegistryImpl>) {
    let codec_registry = Arc::new(CodecFilterRegistryImpl::new());
    let services = PluginServices {
        event_bus: Arc::new(EventBusImpl::new()),
        player_registry: Arc::new(MockPlayerRegistry),
        server_manager: Arc::new(NoopServerManager),
        ban_service: Arc::new(MockBanService),
        command_manager: Arc::new(CommandManagerImpl::new()),
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(MockConfigService),
        load_balancer_service: Arc::new(MockLoadBalancerService),
        plugin_registry: Arc::new(PluginRegistryImpl::new()),
        codec_filter_registry: Arc::clone(&codec_registry),
        transport_filter_registry: Arc::new(TransportFilterRegistryImpl::new()),
        domain_router: Arc::new(DomainRouter::new()),
        proxy_shutdown: tokio_util::sync::CancellationToken::new(),
        proxy_info: infrarust_api::services::proxy_info::ProxyInfo::default(),
        plugins_dir,
    };
    let mut configs = HashMap::new();
    if grant {
        configs.insert(
            plugin_id.to_string(),
            PluginPermissions {
                permissions: vec!["codec-filter".to_string()],
                trusted: false,
            },
        );
    }
    (
        PluginContextFactoryImpl::new(services, configs),
        codec_registry,
    )
}

async fn load_enabled(
    loader: &WasmPluginLoader,
    factory: &PluginContextFactoryImpl,
    id: &str,
) -> Box<dyn Plugin> {
    let plugin = loader
        .load(id, factory)
        .await
        .unwrap_or_else(|e| panic!("load {id}: {e}"));
    let ctx = factory.create_context(id);
    plugin
        .on_enable(ctx.as_ref())
        .await
        .unwrap_or_else(|e| panic!("enable {id}: {e}"));
    plugin
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

#[tokio::test(flavor = "multi_thread")]
async fn codec_filter_modifies_drops_and_injects() {
    let (_tmp, plugins_dir) = stage("codec-modify");
    let loader = fresh_loader();
    let (factory, registry) = codec_env(plugins_dir.clone(), "codec-modify", true);
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &factory, "codec-modify").await;

    assert!(
        !registry.is_empty(),
        "the guest's on_enable registered a codec filter with the host"
    );
    let mut chain = client_chain(&registry);

    // id 0x02 → modified in place.
    let mut packet = RawPacket::new(0x02, Bytes::from_static(b"original"));
    assert!(matches!(chain.process(&mut packet), FilterResult::Pass));
    assert_eq!(
        &packet.data[..],
        b"MODIFIED",
        "guest's in-place packet modification was applied"
    );

    // id 0x01 → dropped.
    let mut packet = RawPacket::new(0x01, Bytes::from_static(b"x"));
    assert!(matches!(chain.process(&mut packet), FilterResult::Dropped));

    // id 0x03 → passes with injected frames before and after.
    let mut packet = RawPacket::new(0x03, Bytes::new());
    match chain.process(&mut packet) {
        FilterResult::PassWithInjections(mut output) => {
            assert_eq!(output.take_before().len(), 1, "one injected 'before' frame");
            assert_eq!(output.take_after().len(), 1, "one injected 'after' frame");
        }
        _ => panic!("expected PassWithInjections for id 0x03"),
    }

    // any other id → unchanged, zero-copy pass.
    let mut packet = RawPacket::new(0x10, Bytes::from_static(b"keep"));
    assert!(matches!(chain.process(&mut packet), FilterResult::Pass));
    assert_eq!(
        &packet.data[..],
        b"keep",
        "unmodified packet passes untouched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn codec_filter_state_is_per_instance() {
    let (_tmp, plugins_dir) = stage("codec-stateful");
    let loader = fresh_loader();
    let (factory, registry) = codec_env(plugins_dir.clone(), "codec-stateful", true);
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &factory, "codec-stateful").await;

    // The client-side and server-side chains hold independent guest instances.
    let (mut client, mut server) = build_codec_chains(
        &registry,
        ProtocolVersion::new(767),
        1,
        "127.0.0.1:1".parse().unwrap(),
        None,
    );

    let mut packet = RawPacket::new(0x00, Bytes::new());
    client.process(&mut packet);
    assert_eq!(&packet.data[..], &1u32.to_le_bytes(), "client count = 1");
    client.process(&mut packet);
    assert_eq!(&packet.data[..], &2u32.to_le_bytes(), "client count = 2");

    let mut other = RawPacket::new(0x00, Bytes::new());
    server.process(&mut other);
    assert_eq!(
        &other.data[..],
        &1u32.to_le_bytes(),
        "server-side instance counts from its own zero"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn codec_filter_denied_without_capability() {
    let (_tmp, plugins_dir) = stage("codec-modify");
    let loader = fresh_loader();
    // Same fixture, but the `codec-filter` capability is NOT granted: the host omits
    // the codec-registry import, so the guest's call leaves an unresolved import and
    // instantiation fails.
    let (factory, _registry) = codec_env(plugins_dir.clone(), "codec-modify", false);
    loader.discover(&plugins_dir).await.unwrap();

    let result = loader.load("codec-modify", &factory).await;
    assert!(
        result.is_err(),
        "registering a codec filter without the capability must fail to load"
    );
}
