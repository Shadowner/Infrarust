#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use infrarust_api::loader::PluginLoader;
use infrarust_api::types::{ProtocolVersion, RawPacket};
use infrarust_core::filter::FilterOwner;
use infrarust_core::filter::codec_chain::{CodecFilterChain, FilterResult, build_codec_chains};
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;
use infrarust_core::plugin::PluginContextFactoryImpl;
use infrarust_core::services::command_manager::DispatchOutcome;
use tracing::Level;
use tracing::instrument::WithSubscriber;

use support::log_capture::LogCapture;
use support::{EnvOptions, console, fresh_loader, load_enabled, make_env_with, stage};

fn codec_env(
    plugins_dir: PathBuf,
    plugin_id: &str,
    grant: bool,
) -> (PluginContextFactoryImpl, Arc<CodecFilterRegistryImpl>) {
    let options = if grant {
        EnvOptions::default().grant(plugin_id, "codec-filter")
    } else {
        EnvOptions::default()
    };
    let env = make_env_with(plugins_dir, options);
    (env.factory, env.codec_registry)
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

    let mut packet = RawPacket::new(0x02, Bytes::from_static(b"original"));
    assert!(matches!(chain.process(&mut packet), FilterResult::Pass));
    assert_eq!(
        &packet.data[..],
        b"MODIFIED",
        "guest's in-place packet modification was applied"
    );

    let mut packet = RawPacket::new(0x01, Bytes::from_static(b"x"));
    assert!(matches!(chain.process(&mut packet), FilterResult::Dropped));

    let mut packet = RawPacket::new(0x03, Bytes::new());
    match chain.process(&mut packet) {
        FilterResult::PassWithInjections(mut output) => {
            assert_eq!(output.take_before().len(), 1, "one injected 'before' frame");
            assert_eq!(output.take_after().len(), 1, "one injected 'after' frame");
        }
        _ => panic!("expected PassWithInjections for id 0x03"),
    }

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
async fn codec_filter_registration_is_refused_without_capability() {
    let (_tmp, plugins_dir) = stage("codec-modify");
    let loader = fresh_loader();
    let (factory, registry) = codec_env(plugins_dir.clone(), "codec-modify", false);
    let logs = LogCapture::at(Level::WARN);

    async {
        loader.discover(&plugins_dir).await.unwrap();
        let _plugin = load_enabled(&loader, &factory, "codec-modify").await;
    }
    .with_subscriber(logs.clone())
    .await;

    assert!(
        registry.is_empty(),
        "the host refused the registration, so no filter joins the chain"
    );
    let report = logs.matching("calls will be refused");
    assert_eq!(report.len(), 1, "{:?}", logs.lines());
    assert!(
        report[0].contains("codec-registry") && report[0].contains("`codec-filter`"),
        "{report:?}"
    );
    let refused = logs.matching("missing capability");
    assert_eq!(refused.len(), 1, "{:?}", logs.lines());
    assert!(
        refused[0].contains("codec-registry.register-codec-filter"),
        "{refused:?}"
    );
}

async fn enable_codec_std(
    plugins_dir: &std::path::Path,
) -> (
    Arc<CodecFilterRegistryImpl>,
    Box<dyn infrarust_api::plugin::Plugin>,
    infrarust_loader_wasm::WasmPluginLoader,
) {
    let loader = fresh_loader();
    let (factory, registry) = codec_env(plugins_dir.to_path_buf(), "codec-std", true);
    loader.discover(plugins_dir).await.unwrap();
    let plugin = load_enabled(&loader, &factory, "codec-std").await;
    (registry, plugin, loader)
}

fn count_of(packet: &RawPacket) -> u32 {
    u32::from_le_bytes(
        packet.data[..]
            .try_into()
            .unwrap_or_else(|_| panic!("the filter did not rewrite {:?}", packet.data)),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn codec_filter_can_log_and_use_std_collections() {
    let (_tmp, plugins_dir) = stage("codec-std");
    let logs = LogCapture::at(Level::INFO);

    let counts = async {
        let (registry, _plugin, _loader) = enable_codec_std(&plugins_dir).await;
        let mut chain = client_chain(&registry);
        let mut counts = Vec::new();
        for id in [0x05, 0x05, 0x06, 0x05] {
            let mut packet = RawPacket::new(id, Bytes::new());
            assert!(matches!(chain.process(&mut packet), FilterResult::Pass));
            counts.push(count_of(&packet));
        }
        counts
    }
    .with_subscriber(logs.clone())
    .await;

    assert_eq!(
        counts,
        [1, 2, 1, 3],
        "the HashMap keeps a count per packet id across calls"
    );
    assert_eq!(
        logs.matching("codec-std filter created").len(),
        2,
        "{:?}",
        logs.lines()
    );
    let seen = logs.matching("codec-std saw packet");
    assert_eq!(seen.len(), 4, "{:?}", logs.lines());
    assert!(
        seen.iter()
            .all(|line| line.starts_with("INFO ") && line.contains("plugin=codec-std")),
        "{seen:?}"
    );
    assert!(
        logs.matching("trapped").is_empty() && logs.matching("create failed").is_empty(),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn codec_filter_logging_is_rate_limited_per_plugin() {
    const PACKETS: u32 = 500;
    let (_tmp, plugins_dir) = stage("codec-std");
    let logs = LogCapture::at(Level::INFO);

    let last = async {
        let (registry, _plugin, _loader) = enable_codec_std(&plugins_dir).await;
        let mut chain = client_chain(&registry);
        let mut last = 0;
        for _ in 0..PACKETS {
            let mut packet = RawPacket::new(0x07, Bytes::new());
            chain.process(&mut packet);
            last = count_of(&packet);
        }
        last
    }
    .with_subscriber(logs.clone())
    .await;

    assert_eq!(last, PACKETS, "every packet still went through the guest");
    let seen = logs.matching("codec-std saw packet").len();
    let limit = usize::try_from(PACKETS / 2).unwrap();
    assert!(
        (1..limit).contains(&seen),
        "{seen} of {PACKETS} filter log lines reached the proxy log"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_a_wasm_plugin_removes_its_codec_filter() {
    let (_tmp, plugins_dir) = stage("codec-modify");
    let loader = fresh_loader();
    let (factory, registry) = codec_env(plugins_dir.clone(), "codec-modify", true);
    loader.discover(&plugins_dir).await.unwrap();
    let plugin = load_enabled(&loader, &factory, "codec-modify").await;
    assert_eq!(
        registry.owner_of("ops"),
        Some(FilterOwner::plugin("codec-modify"))
    );

    plugin.on_disable().await.unwrap();

    assert!(
        registry.is_empty(),
        "the stopped plugin's filter no longer joins new connections"
    );
    let mut packet = RawPacket::new(0x02, Bytes::from_static(b"original"));
    assert!(matches!(
        client_chain(&registry).process(&mut packet),
        FilterResult::Pass
    ));
    assert_eq!(&packet.data[..], b"original");
}

#[tokio::test(flavor = "multi_thread")]
async fn unloading_a_wasm_plugin_removes_its_codec_filter() {
    let (_tmp, plugins_dir) = stage("codec-modify");
    let loader = fresh_loader();
    let (factory, registry) = codec_env(plugins_dir.clone(), "codec-modify", true);
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &factory, "codec-modify").await;
    assert!(!registry.is_empty());

    loader.unload("codec-modify").await.unwrap();

    assert!(registry.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_codec_filter_survives_a_recovery_of_its_plugin() {
    let (_tmp, plugins_dir) = stage("codec-std");
    let loader = fresh_loader();
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions::default().grant("codec-std", "codec-filter"),
    );
    let registry = Arc::clone(&env.codec_registry);
    let logs = LogCapture::at(Level::INFO);

    let (owner, count, left_after_unload) = async {
        loader.discover(&plugins_dir).await.unwrap();
        let _plugin = load_enabled(&loader, &env.factory, "codec-std").await;
        assert_eq!(
            env.command_manager
                .dispatch(console(), "codec-std-trap")
                .await,
            DispatchOutcome::Executed
        );
        let mut packet = RawPacket::new(0x05, Bytes::new());
        client_chain(&registry).process(&mut packet);
        let owner = registry.owner_of("tally");
        loader.unload("codec-std").await.unwrap();
        (owner, count_of(&packet), registry.owned_by("codec-std"))
    }
    .with_subscriber(logs.clone())
    .await;

    let recovered = logs.matching("wasm plugin recovered");
    assert_eq!(recovered.len(), 1, "{:?}", logs.lines());
    assert!(recovered[0].contains("generation=2"), "{recovered:?}");
    assert!(
        logs.matching("codec filter registration").is_empty(),
        "the fresh instance registers its own filter id again: {:?}",
        logs.lines()
    );
    assert_eq!(owner, Some(FilterOwner::plugin("codec-std")));
    assert_eq!(
        count, 1,
        "a connection opened after the recovery runs the filter"
    );
    assert!(
        left_after_unload.is_empty(),
        "unloading the recovered plugin still removes the filter: {left_after_unload:?}"
    );
}
