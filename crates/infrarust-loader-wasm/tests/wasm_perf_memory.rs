#![cfg(all(feature = "wasm", wasm_fixtures_available, target_os = "linux"))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use infrarust_api::loader::PluginLoader;
use infrarust_api::types::ProtocolVersion;
use infrarust_core::filter::codec_chain::build_codec_chains;

use support::{EnvOptions, load_enabled, loader_from_toml, make_env_with, stage};

const CONNECTIONS: usize = 256;
const POOL: &str = "[wasm]\ninstance_pool = 1024\n";

const THP_ENABLED: &str = "/sys/kernel/mm/transparent_hugepage/enabled";

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn thp_always() -> bool {
    let selected = std::fs::read_to_string(THP_ENABLED).is_ok_and(|mode| mode.contains("[always]"));
    if !selected {
        eprintln!("{THP_ENABLED} does not select [always]; this host cannot show the regression");
    }
    selected
}

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap()
}

struct Resident {
    open_per_connection: f64,
    left_per_connection: f64,
}

async fn resident_per_connection(proxy_toml: &str) -> Resident {
    let (_tmp, plugins_dir) = stage("codec-modify");
    let loader = loader_from_toml(proxy_toml);
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions::default().grant("codec-modify", "codec-filter"),
    );
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "codec-modify").await;
    assert!(!env.codec_registry.is_empty());

    let before = rss_kib();
    let open: Vec<_> = (0..CONNECTIONS)
        .map(|n| {
            build_codec_chains(
                &env.codec_registry,
                ProtocolVersion::new(767),
                n as u64,
                "127.0.0.1:1".parse().unwrap(),
                None,
            )
        })
        .collect();
    let open_per_connection = rss_kib().saturating_sub(before) as f64 / CONNECTIONS as f64;
    for (mut client, mut server) in open {
        client.close();
        server.close();
    }
    let left_per_connection = rss_kib().saturating_sub(before) as f64 / CONNECTIONS as f64;
    Resident {
        open_per_connection,
        left_per_connection,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-12: instance_pool pins huge pages per connection"]
async fn under_thp_always_a_pooled_codec_instance_is_not_much_heavier_than_an_on_demand_one() {
    if !thp_always() {
        return;
    }
    let _serial = SERIAL.lock().await;
    let on_demand = resident_per_connection("").await;
    let pooled = resident_per_connection(POOL).await;
    assert!(
        pooled.open_per_connection <= on_demand.open_per_connection * 2.0 + 16.0,
        "a connection costs {:.0} KiB resident with instance_pool against {:.0} KiB without; \
         the pool should reserve address space, not memory (see /proc/self/smaps AnonHugePages \
         on the pooled table region)",
        pooled.open_per_connection,
        on_demand.open_per_connection
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-12: instance_pool pins huge pages per connection"]
async fn under_thp_always_closing_pooled_codec_connections_gives_their_memory_back() {
    if !thp_always() {
        return;
    }
    let _serial = SERIAL.lock().await;
    let pooled = resident_per_connection(POOL).await;
    assert!(
        pooled.left_per_connection <= 16.0,
        "{:.0} KiB per closed connection is still resident after every connection closed \
         ({:.0} KiB while open)",
        pooled.left_per_connection,
        pooled.open_per_connection
    );
}
