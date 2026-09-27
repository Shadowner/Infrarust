#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/net.rs"]
mod net;
mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_api::plugin::Plugin;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::test_util::MockConfigService;
use infrarust_core::services::command_manager::DispatchOutcome;
use net::{UdpSink, enable_probe, free_port, network_toml};
use support::{EnvOptions, TestEnv, add_fixture, console, loader_from_toml, make_env_with, stage};

const DENIED: &str = "err PermissionDenied";
const SEC: &str = "sec-probe";

struct SecProbe {
    _tmp: tempfile::TempDir,
    data: PathBuf,
    env: TestEnv,
    _plugin: Box<dyn Plugin>,
    next: AtomicUsize,
}

async fn enable_sec(grants: &[&str], proxy_toml: &str) -> SecProbe {
    let (tmp, plugins_dir) = stage(SEC);
    let options = grants
        .iter()
        .fold(EnvOptions::default(), |options, grant| {
            options.grant(SEC, grant)
        });
    let env = make_env_with(plugins_dir.clone(), options);
    let loader = loader_from_toml(proxy_toml);
    loader.discover(&plugins_dir).await.unwrap();
    let plugin = loader.load(SEC, &env.factory).await.unwrap();
    let ctx = env.factory.create_context(SEC);
    plugin.on_enable(ctx.as_ref()).await.unwrap();
    SecProbe {
        data: plugins_dir.join(SEC),
        _tmp: tmp,
        env,
        _plugin: plugin,
        next: AtomicUsize::new(1),
    }
}

impl SecProbe {
    async fn run(&self, line: &str) -> String {
        let tag = format!("t{}", self.next.fetch_add(1, Ordering::Relaxed));
        let outcome = tokio::time::timeout(
            Duration::from_secs(30),
            self.env
                .command_manager
                .dispatch(console(), &format!("sec {tag} {line}")),
        )
        .await
        .expect("a sec probe answers promptly");
        assert_eq!(outcome, DispatchOutcome::Executed, "sec {line}");
        let prefix = format!("{tag} ");
        std::fs::read_to_string(self.data.join("sec.log"))
            .unwrap_or_default()
            .lines()
            .find_map(|entry| entry.strip_prefix(&prefix).map(str::to_owned))
            .unwrap_or_else(|| format!("no outcome for `{line}`"))
    }

    fn plugin_command_count(&self) -> usize {
        self.env.command_manager.commands_for_plugin(SEC).len()
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-06: UdpReceive and TcpAccept always allowed"]
async fn a_udp_socket_cannot_connect_to_a_denied_address() {
    let allowed = UdpSink::start().await;
    let denied = UdpSink::start().await;
    let probe = enable_probe(&["network"], &network_toml(&[allowed.addr.to_string()], "")).await;

    assert_eq!(
        probe.run(&format!("udp-connect {}", denied.addr)).await,
        DENIED,
        "connecting a UDP socket to an address outside the allow-list must be refused"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-06: UdpReceive and TcpAccept always allowed"]
async fn an_ephemeral_udp_socket_does_not_receive_from_an_unlisted_source() {
    let allowed = UdpSink::start().await;
    let probe = std::sync::Arc::new(
        enable_probe(&["network"], &network_toml(&[allowed.addr.to_string()], "")).await,
    );

    let receiver = std::sync::Arc::clone(&probe);
    let recv = tokio::spawn(async move { receiver.run("udp-recv 0.0.0.0:0").await });

    let port_file = probe.data.join("udp.port");
    let mut bound = String::new();
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(&port_file)
            && !text.is_empty()
        {
            bound = text;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let port = bound
        .rsplit_once(':')
        .map(|(_, port)| port.to_owned())
        .unwrap_or_default();
    assert!(!port.is_empty(), "the guest reported its bound port: {bound:?}");

    let source = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    for _ in 0..20 {
        source
            .send_to(b"unsolicited", format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let outcome = tokio::time::timeout(Duration::from_secs(5), recv).await;
    match outcome {
        Ok(Ok(line)) => assert!(
            !line.contains("unsolicited"),
            "an ephemeral UDP socket received a datagram from a source no allow-list rule covers: {line}"
        ),
        _ => {}
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_cannot_listen_on_ipv6_by_default() {
    let port = free_port();
    let probe = enable_probe(&["network"], &network_toml(&["[::/0]:*".to_owned()], "")).await;
    assert_eq!(probe.run("listen [::1]:0").await, DENIED);
    assert_eq!(probe.run(&format!("listen [::1]:{port}")).await, DENIED);
    assert_eq!(probe.run("listen [::]:0").await, DENIED);
    assert_eq!(probe.run(&format!("listen [::]:{port}")).await, DENIED);
    assert_eq!(probe.run(&format!("udp-bind [::1]:{port}")).await, DENIED);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exact_ipv6_rule_lets_a_plugin_listen_on_that_address() {
    let port = free_port();
    let probe = enable_probe(
        &["network"],
        &network_toml(&[format!("[::1]:{port}")], ""),
    )
    .await;
    assert_eq!(
        probe.run(&format!("listen [::1]:{port}")).await,
        format!("ok [::1]:{port}")
    );
    assert_eq!(probe.run("listen [::1]:0").await, DENIED);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ephemeral_bind_does_not_let_a_plugin_listen() {
    let probe = enable_probe(&["network"], &network_toml(&["0.0.0.0/0:*".to_owned()], "")).await;
    let bound = probe.run("udp-bind 0.0.0.0:0").await;
    assert!(bound.starts_with("ok "), "an ephemeral UDP bind is allowed: {bound}");
    assert_eq!(probe.run("listen 0.0.0.0:0").await, DENIED);
    assert_eq!(probe.run("listen [::]:0").await, DENIED);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-07: no per-plugin registration quota"]
async fn the_command_registry_bounds_how_many_a_plugin_registers() {
    let flood = 3000;
    let probe = enable_sec(&[], "").await;
    let reported = probe.run(&format!("commands {flood}")).await;
    let host_count = probe.plugin_command_count();
    assert!(
        host_count <= flood / 2,
        "a plugin registered {reported} commands and the host kept {host_count}; a per-plugin cap should stop unbounded growth"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-07: no per-plugin registration quota"]
async fn the_plugin_channel_registry_bounds_how_many_a_plugin_registers() {
    let flood = 3000;
    let probe = enable_sec(&["plugin-messaging"], "").await;
    let reported = probe.run(&format!("channels {flood}")).await;
    let held: usize = reported
        .split_whitespace()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    assert!(
        held <= flood / 2,
        "a plugin registered {reported} plugin channels into the shared registry; a per-plugin cap should stop unbounded growth"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-07: no per-plugin registration quota"]
async fn the_scheduler_bounds_how_many_tasks_a_plugin_registers() {
    let flood = 3000;
    let probe = enable_sec(&[], "").await;
    let reported = probe.run(&format!("tasks {flood}")).await;
    let scheduled: usize = reported
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    assert!(
        scheduled <= flood / 2,
        "a plugin scheduled {scheduled} tasks (each a live host task); a per-plugin cap should stop unbounded growth"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deeply_nested_json_does_not_crash_the_host_parser() {
    let probe = enable_sec(&[], "").await;
    let deep = probe.run("json-nest 20000").await;
    assert!(
        deep.starts_with("ok") || deep.starts_with("err"),
        "the host survived a deeply nested text component: {deep}"
    );
    assert_eq!(
        probe.run("forge-get").await,
        "ok absent",
        "the host still answers after the nested parse"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_huge_json_payload_does_not_crash_the_host_parser() {
    let probe = enable_sec(&[], "").await;
    let huge = probe.run("json-huge 50000").await;
    assert!(
        huge.starts_with("ok plain-len"),
        "the host parsed a huge payload: {huge}"
    );
    assert_eq!(probe.run("forge-get").await, "ok absent");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forged_player_id_reads_as_absent() {
    let probe = enable_sec(&[], "").await;
    assert_eq!(probe.run("forge-get").await, "ok absent");
}

struct World {
    _tmp: tempfile::TempDir,
    plugins_dir: PathBuf,
    env: TestEnv,
    _plugins: Vec<Box<dyn Plugin>>,
    next: AtomicUsize,
}

async fn enable_world(specs: &[(&str, &[&str])]) -> World {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    let mut options = EnvOptions::default();
    for (id, grants) in specs {
        add_fixture(&plugins_dir, id, id);
        for grant in *grants {
            options = options.grant(id, grant);
        }
    }
    let env = make_env_with(plugins_dir.clone(), options);
    let loader = loader_from_toml("");
    loader.discover(&plugins_dir).await.unwrap();
    let mut plugins = Vec::new();
    for (id, _) in specs {
        let plugin = loader.load(id, &env.factory).await.unwrap();
        let ctx = env.factory.create_context(id);
        plugin.on_enable(ctx.as_ref()).await.unwrap();
        plugins.push(plugin);
    }
    World {
        _tmp: tmp,
        plugins_dir,
        env,
        _plugins: plugins,
        next: AtomicUsize::new(1),
    }
}

impl World {
    async fn run_sec(&self, line: &str) -> String {
        let tag = format!("t{}", self.next.fetch_add(1, Ordering::Relaxed));
        let outcome = tokio::time::timeout(
            Duration::from_secs(30),
            self.env
                .command_manager
                .dispatch(console(), &format!("sec {tag} {line}")),
        )
        .await
        .expect("a sec probe answers promptly");
        assert_eq!(outcome, DispatchOutcome::Executed, "sec {line}");
        let prefix = format!("{tag} ");
        std::fs::read_to_string(self.plugins_dir.join(SEC).join("sec.log"))
            .unwrap_or_default()
            .lines()
            .find_map(|entry| entry.strip_prefix(&prefix).map(str::to_owned))
            .unwrap_or_else(|| format!("no outcome for `{line}`"))
    }

    async fn dispatch(&self, line: &str) {
        let outcome = tokio::time::timeout(
            Duration::from_secs(30),
            self.env.command_manager.dispatch(console(), line),
        )
        .await
        .expect("a command answers promptly");
        assert_eq!(outcome, DispatchOutcome::Executed, "{line}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_cannot_unregister_another_plugins_command() {
    let world = enable_world(&[(SEC, &[]), ("command-plugin", &[])]).await;
    assert!(
        world
            .env
            .command_manager
            .commands_for_plugin("command-plugin")
            .iter()
            .any(|info| info.name() == "greet"),
        "command-plugin owns greet"
    );

    for target in ["greet", "command-plugin:greet"] {
        let outcome = world.run_sec(&format!("forge-unregister {target}")).await;
        assert!(
            outcome.starts_with("err "),
            "unregistering another plugin's command must fail: {target} -> {outcome}"
        );
    }

    assert!(
        world
            .env
            .command_manager
            .commands_for_plugin("command-plugin")
            .iter()
            .any(|info| info.name() == "greet"),
        "command-plugin still owns greet after the forged unregister attempts"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_cannot_unregister_another_plugins_codec_filter() {
    let world = enable_world(&[(SEC, &["codec-filter"]), ("net-codec", &["codec-filter"])]).await;
    assert!(
        world.env.codec_registry.owned_by("net-codec").contains(&"reach".to_owned()),
        "net-codec owns the reach filter"
    );

    let outcome = world.run_sec("forge-unreg-codec reach").await;
    assert!(
        outcome.starts_with("err "),
        "unregistering another plugin's codec filter must fail: {outcome}"
    );
    assert!(
        world.env.codec_registry.owned_by("net-codec").contains(&"reach".to_owned()),
        "net-codec still owns reach after the forged unregister attempt"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_cannot_unregister_another_plugins_channel() {
    let world = enable_world(&[
        (SEC, &["plugin-messaging"]),
        ("sec-peer", &["plugin-messaging"]),
    ])
    .await;

    let attempt = world.run_sec("forge-unreg-channel sec:peer").await;
    assert_eq!(
        attempt, "ok false",
        "unregistering another plugin's channel must remove nothing: {attempt}"
    );

    world.dispatch("peer-cmd channels").await;
    let peer_view = std::fs::read_to_string(world.plugins_dir.join("sec-peer").join("peer.log"))
        .unwrap_or_default();
    assert!(
        peer_view.lines().last() == Some("1"),
        "sec-peer still holds its channel after the forged unregister: {peer_view:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forged_listener_handle_is_a_safe_no_op() {
    let probe = enable_sec(&[], "").await;
    assert_eq!(
        probe.run("forge-unsub 999999").await,
        "ok false",
        "unsubscribing a handle the plugin never held removes nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forged_scheduler_handle_is_a_safe_no_op() {
    let probe = enable_sec(&[], "").await;
    assert_eq!(
        probe.run("forge-cancel 999999").await,
        "ok cancelled",
        "cancelling a handle the plugin never held is a safe no-op"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-07: no per-plugin registration quota"]
async fn the_event_bus_bounds_how_many_listeners_a_plugin_registers() {
    let flood = 3000;
    let probe = enable_sec(&[], "").await;
    let reported = probe.run(&format!("listeners {flood}")).await;
    let registered: usize = reported
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    assert!(
        registered <= flood / 2,
        "a plugin registered {registered} event listeners; a per-plugin cap should stop unbounded growth"
    );
}

async fn enable_sec_with_config(config: MockConfigService) -> SecProbe {
    let (tmp, plugins_dir) = stage(SEC);
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions {
            config_service: std::sync::Arc::new(config) as std::sync::Arc<dyn ConfigService>,
            ..EnvOptions::default()
        },
    );
    let loader = loader_from_toml("");
    loader.discover(&plugins_dir).await.unwrap();
    let plugin = loader.load(SEC, &env.factory).await.unwrap();
    let ctx = env.factory.create_context(SEC);
    plugin.on_enable(ctx.as_ref()).await.unwrap();
    SecProbe {
        data: plugins_dir.join(SEC),
        _tmp: tmp,
        env,
        _plugin: plugin,
        next: AtomicUsize::new(1),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-16: config-read reads every other plugin's config"]
async fn a_baseline_plugin_reads_every_other_plugins_config() {
    let document = "[plugins.secret-plugin] api_token=TOP-SECRET-TOKEN";
    let config = MockConfigService::new()
        .with_proxy_document(document)
        .with_value("plugins.secret-plugin.api_token", "TOP-SECRET-TOKEN");
    let probe = enable_sec_with_config(config).await;

    let dump = probe.run("config-dump").await;
    assert!(
        !dump.contains("TOP-SECRET-TOKEN"),
        "config-read is baseline, yet a plugin dumped another plugin's secret from the whole proxy config: {dump}"
    );
    let value = probe.run("config-get plugins.secret-plugin.api_token").await;
    assert!(
        !value.contains("TOP-SECRET-TOKEN"),
        "a plugin read another plugin's config value: {value}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn writing_the_proxy_config_needs_config_write() {
    let probe = enable_sec(&[], "").await;
    let outcome = probe.run("config-write [proxy]\\nbind=\\\"0.0.0.0:1\\\"").await;
    assert_eq!(
        outcome, "err ErrorKind::PermissionDenied",
        "write-proxy-config-document without config-write must be refused"
    );
}
