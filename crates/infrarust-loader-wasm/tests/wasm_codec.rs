#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use infrarust_api::loader::PluginLoader;
use infrarust_api::types::{ProtocolVersion, RawPacket};
use infrarust_core::filter::codec_chain::{CodecFilterChain, FilterResult, build_codec_chains};
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;
use infrarust_core::plugin::PluginContextFactoryImpl;

use support::{EnvOptions, fresh_loader, load_enabled, make_env_with, stage};

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
async fn codec_filter_denied_without_capability() {
    let (_tmp, plugins_dir) = stage("codec-modify");
    let loader = fresh_loader();
    let (factory, _registry) = codec_env(plugins_dir.clone(), "codec-modify", false);
    loader.discover(&plugins_dir).await.unwrap();

    let result = loader.load("codec-modify", &factory).await;
    assert!(
        result.is_err(),
        "registering a codec filter without the capability must fail to load"
    );
}
