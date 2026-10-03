#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/net.rs"]
mod net;
mod support;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_api::plugin::Plugin;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::test_util::MockConfigService;
use infrarust_core::routing::DomainRouter;
use infrarust_core::services::command_manager::DispatchOutcome;
use infrarust_core::services::config_service::ConfigServiceImpl;
use net::{UdpSink, enable_probe, free_port, network_toml};
use support::log_capture::LogCapture;
use support::{EnvOptions, TestEnv, add_fixture, console, loader_from_toml, make_env_with, stage};
use tokio::io::AsyncReadExt;
use tracing::Level;
use tracing::instrument::WithSubscriber;

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
    let options = grants.iter().fold(EnvOptions::default(), |options, grant| {
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
    assert!(
        !port.is_empty(),
        "the guest reported its bound port: {bound:?}"
    );

    let source = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    for _ in 0..20 {
        source
            .send_to(b"unsolicited", format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let outcome = tokio::time::timeout(Duration::from_secs(5), recv).await;
    if let Ok(Ok(line)) = outcome {
        assert!(
            !line.contains("unsolicited"),
            "an ephemeral UDP socket received a datagram from a source no allow-list rule covers: {line}"
        );
    }
}

async fn bound_port(file: &Path) -> u16 {
    for _ in 0..400 {
        if let Ok(text) = std::fs::read_to_string(file)
            && let Some((_, port)) = text.rsplit_once(':')
            && let Ok(port) = port.parse()
        {
            return port;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "the guest never reported its bound port in {}",
        file.display()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ephemeral_udp_socket_receives_from_a_listed_source() {
    let source = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let source_addr = source.local_addr().unwrap();
    let probe = std::sync::Arc::new(
        enable_probe(&["network"], &network_toml(&[source_addr.to_string()], "")).await,
    );

    let receiver = std::sync::Arc::clone(&probe);
    let recv = tokio::spawn(async move { receiver.run("udp-recv 0.0.0.0:0").await });
    let port = bound_port(&probe.data.join("udp.port")).await;

    for _ in 0..100 {
        if recv.is_finished() {
            break;
        }
        source
            .send_to(b"listed", ("127.0.0.1", port))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let line = tokio::time::timeout(Duration::from_secs(5), recv)
        .await
        .expect("the guest received a datagram from a listed source")
        .unwrap();
    assert_eq!(line, format!("ok {source_addr} listed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listener_only_accepts_peers_the_allow_list_covers() {
    let listen_port = free_port();
    let listed_port = free_port();
    let logs = LogCapture::at(Level::WARN);

    let (accepted, unlisted_addr, greeting) = async {
        let probe = std::sync::Arc::new(
            enable_probe(
                &["network"],
                &network_toml(
                    &[
                        format!("127.0.0.1:{listen_port}"),
                        format!("127.0.0.1:{listed_port}"),
                    ],
                    "",
                ),
            )
            .await,
        );
        let acceptor = std::sync::Arc::clone(&probe);
        let accept = tokio::spawn(async move {
            acceptor
                .run(&format!("accept 127.0.0.1:{listen_port}"))
                .await
        });
        let port = bound_port(&probe.data.join("tcp.port")).await;
        assert_eq!(port, listen_port);
        let listener: SocketAddr = format!("127.0.0.1:{listen_port}").parse().unwrap();

        let unlisted = loop {
            let socket = tokio::net::TcpSocket::new_v4().unwrap();
            socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
            if socket.local_addr().unwrap().port() != listed_port {
                break socket;
            }
        };
        let unlisted_addr = unlisted.local_addr().unwrap();
        let mut dropped = Vec::new();
        match unlisted.connect(listener).await {
            Ok(mut unlisted) => {
                let _ = tokio::time::timeout(
                    Duration::from_secs(10),
                    unlisted.read_to_end(&mut dropped),
                )
                .await
                .expect("the host drops a peer the allow-list does not cover");
            }
            Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset, "{error}"),
        }
        assert!(
            dropped.is_empty(),
            "a peer no rule covers reached the guest: {dropped:?}"
        );
        assert!(!accept.is_finished(), "the guest is still waiting");

        let listed = tokio::net::TcpSocket::new_v4().unwrap();
        listed.set_reuseaddr(true).unwrap();
        listed
            .bind(format!("127.0.0.1:{listed_port}").parse().unwrap())
            .unwrap();
        let mut listed = listed.connect(listener).await.unwrap();
        let mut greeting = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(10), listed.read_to_end(&mut greeting))
            .await
            .expect("the guest answers a listed peer");
        let accepted = tokio::time::timeout(Duration::from_secs(10), accept)
            .await
            .expect("the accept returns once a listed peer connects")
            .unwrap();
        (accepted, unlisted_addr, greeting)
    }
    .with_subscriber(logs.clone())
    .await;

    assert_eq!(accepted, format!("ok 127.0.0.1:{listed_port}"));
    assert_eq!(greeting, b"accepted");
    let refused = logs.matching("kind=\"tcp-accept\"");
    assert_eq!(refused.len(), 1, "{:?}", logs.lines());
    assert!(
        refused[0].contains("plugin=net-probe")
            && refused[0].contains(&format!("source={unlisted_addr}"))
            && refused[0].contains("reason=\"network allow-list\""),
        "{refused:?}"
    );
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
    let probe = enable_probe(&["network"], &network_toml(&[format!("[::1]:{port}")], "")).await;
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
    assert!(
        bound.starts_with("ok "),
        "an ephemeral UDP bind is allowed: {bound}"
    );
    assert_eq!(probe.run("listen 0.0.0.0:0").await, DENIED);
    assert_eq!(probe.run("listen [::]:0").await, DENIED);
}

#[tokio::test(flavor = "multi_thread")]
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
        world
            .env
            .codec_registry
            .owned_by("net-codec")
            .contains(&"reach".to_owned()),
        "net-codec owns the reach filter"
    );

    let outcome = world.run_sec("forge-unreg-codec reach").await;
    assert!(
        outcome.starts_with("err "),
        "unregistering another plugin's codec filter must fail: {outcome}"
    );
    assert!(
        world
            .env
            .codec_registry
            .owned_by("net-codec")
            .contains(&"reach".to_owned()),
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

const SMALL_QUOTAS: &str = "[wasm.quotas]
event_listeners = 5
commands = 5
scheduled_tasks = 5
plugin_channels = 5
codec_filters = 5
limbo_handlers = 5
";

#[tokio::test(flavor = "multi_thread")]
async fn each_registration_quota_refuses_with_limit_exceeded_and_a_release_frees_room() {
    let probe = enable_sec(&["plugin-messaging", "codec-filter", "limbo"], SMALL_QUOTAS).await;
    for (family, expected) in [
        ("listeners", "ok 5 ErrorKind::LimitExceeded retry=ok"),
        ("commands", "ok 4 ErrorKind::LimitExceeded retry=ok"),
        ("tasks", "ok 5 ErrorKind::LimitExceeded retry=ok"),
        ("channels", "ok 5 ErrorKind::LimitExceeded retry=ok"),
        ("codecs", "ok 5 ErrorKind::LimitExceeded retry=ok"),
        ("limbo", "ok 5 ErrorKind::LimitExceeded retry=none"),
    ] {
        assert_eq!(
            probe.run(&format!("quota {family} 20")).await,
            expected,
            "{family}: the plugin's own `sec` command counts toward the command quota"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_quota_override_replaces_the_proxy_wide_quota() {
    let config = format!("{SMALL_QUOTAS}\n[plugins.sec-probe.wasm.quotas]\nscheduled_tasks = 12\n");
    let probe = enable_sec(&[], &config).await;
    assert_eq!(
        probe.run("quota tasks 20").await,
        "ok 12 ErrorKind::LimitExceeded retry=ok"
    );
    assert_eq!(
        probe.run("quota listeners 20").await,
        "ok 5 ErrorKind::LimitExceeded retry=ok"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_quota_refusal_is_logged_once_per_window_naming_the_plugin_and_the_quota() {
    let logs = LogCapture::at(Level::WARN);
    async {
        let probe = enable_sec(&[], SMALL_QUOTAS).await;
        assert_eq!(
            probe.run("quota listeners 200").await,
            "ok 5 ErrorKind::LimitExceeded retry=ok"
        );
    }
    .with_subscriber(logs.clone())
    .await;
    let refusals = logs.matching("registration refused");
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert!(
        refusals[0].contains("sec-probe") && refusals[0].contains("quotas.event_listeners"),
        "{}",
        refusals[0]
    );
}

async fn enable_sec_with_config(
    grants: &[&str],
    config: std::sync::Arc<dyn ConfigService>,
) -> SecProbe {
    let (tmp, plugins_dir) = stage(SEC);
    let options = EnvOptions {
        config_service: config,
        ..EnvOptions::default()
    };
    let options = grants
        .iter()
        .fold(options, |options, grant| options.grant(SEC, grant));
    let env = make_env_with(plugins_dir.clone(), options);
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

const OTHER_SECRET: &str = "TOP-SECRET-TOKEN";
const OTHER_BLOCK: &str = "[plugins.secret-plugin]\napi_token = \"TOP-SECRET-TOKEN\"\n";
const SEC_DENIED: &str = "err ErrorKind::PermissionDenied";

fn proxy_document() -> String {
    format!("bind = \"0.0.0.0:25565\"\n\n{OTHER_BLOCK}\n[plugins.sec-probe]\nenabled = true\n")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_baseline_plugin_cannot_read_another_plugins_config() {
    let config = MockConfigService::new()
        .with_proxy_document(&proxy_document())
        .with_value("plugins.secret-plugin.api_token", OTHER_SECRET);
    let probe = enable_sec_with_config(&[], std::sync::Arc::new(config)).await;

    let dump = probe.run("config-dump").await;
    assert!(dump.starts_with("ok "), "{dump}");
    assert!(
        !dump.contains(OTHER_SECRET) && !dump.contains("secret-plugin"),
        "config-read is baseline, yet a plugin read another plugin's block from the proxy config: {dump}"
    );
    assert!(
        dump.contains("[plugins.sec-probe]") && dump.contains("0.0.0.0:25565"),
        "the plugin's own block and the rest of the document stay readable: {dump}"
    );
    assert_eq!(
        probe
            .run("config-get plugins.secret-plugin.api_token")
            .await,
        SEC_DENIED,
        "a plugin read another plugin's config value"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_value_under_another_plugin_is_refused_whether_or_not_it_exists() {
    let config = MockConfigService::new()
        .with_value(
            "plugins.secret-plugin",
            "{ api_token = \"TOP-SECRET-TOKEN\" }",
        )
        .with_value("plugins.secret-plugin.api_token", OTHER_SECRET);
    let probe = enable_sec_with_config(&[], std::sync::Arc::new(config)).await;

    for key in [
        "plugins.secret-plugin",
        "plugins.secret-plugin.api_token",
        "plugins.nobody.path",
        "plugins.sec-probe-twin.path",
    ] {
        assert_eq!(
            probe.run(&format!("config-get {key}")).await,
            SEC_DENIED,
            "{key}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_reads_its_own_values_and_the_rest_of_the_config() {
    let config = MockConfigService::new()
        .with_value("bind", "0.0.0.0:25565")
        .with_value("plugins.sec-probe.greeting", "hello")
        .with_value(
            "plugins",
            "{ secret-plugin = { api_token = \"TOP-SECRET-TOKEN\" }, sec-probe = { greeting = \"hello\" } }",
        );
    let probe = enable_sec_with_config(&[], std::sync::Arc::new(config)).await;

    assert_eq!(probe.run("config-get bind").await, "ok 0.0.0.0:25565");
    assert_eq!(
        probe.run("config-get plugins.sec-probe.greeting").await,
        "ok hello"
    );
    assert_eq!(
        probe.run("config-get plugins.sec-probe.missing").await,
        "ok none"
    );
    assert_eq!(
        probe.run("config-get plugins").await,
        "ok { sec-probe = { greeting = \"hello\" } }",
        "the whole plugins table keeps only the caller's entry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_effective_document_hides_other_plugins_blocks() {
    let config = MockConfigService::new()
        .with_proxy_document("bind = \"0.0.0.0:25565\"\n")
        .effective_with(|document| {
            format!("{document}\n{OTHER_BLOCK}\n[plugins.sec-probe]\nenabled = true\n")
        });
    let probe = enable_sec_with_config(&[], std::sync::Arc::new(config)).await;

    let effective = probe.run("config-effective").await;
    assert!(effective.starts_with("ok "), "{effective}");
    assert!(
        !effective.contains(OTHER_SECRET) && !effective.contains("secret-plugin"),
        "{effective}"
    );
    assert!(
        effective.contains("[plugins.sec-probe]") && effective.contains("0.0.0.0:25565"),
        "{effective}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_document_the_host_cannot_parse_is_not_handed_out() {
    let config = MockConfigService::new()
        .with_proxy_document("[plugins.secret-plugin] api_token=TOP-SECRET-TOKEN");
    let probe = enable_sec_with_config(&[], std::sync::Arc::new(config)).await;

    assert_eq!(probe.run("config-dump").await, "err ErrorKind::Internal");
}

#[tokio::test(flavor = "multi_thread")]
async fn writing_the_config_back_keeps_the_blocks_the_plugin_cannot_see() {
    let config = std::sync::Arc::new(
        MockConfigService::new()
            .with_proxy_document(&proxy_document())
            .accepting_writes(),
    );
    let probe = enable_sec_with_config(
        &["config-write"],
        std::sync::Arc::<MockConfigService>::clone(&config),
    )
    .await;

    assert_eq!(
        probe
            .run("config-write bind=\"0.0.0.0:1\"\\n[plugins.sec-probe]\\nenabled=false")
            .await,
        "ok written"
    );

    let stored: toml::Table = toml::from_str(&config.stored_proxy_document()).unwrap();
    assert_eq!(stored["bind"].as_str(), Some("0.0.0.0:1"));
    assert_eq!(
        stored["plugins"]["sec-probe"]["enabled"].as_bool(),
        Some(false)
    );
    assert_eq!(
        stored["plugins"]["secret-plugin"]["api_token"].as_str(),
        Some(OTHER_SECRET),
        "a plugin that cannot see another plugin's block must not erase it by writing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn writing_another_plugins_block_is_refused() {
    let config = std::sync::Arc::new(
        MockConfigService::new()
            .with_proxy_document(&proxy_document())
            .accepting_writes(),
    );
    let probe = enable_sec_with_config(
        &["config-write"],
        std::sync::Arc::<MockConfigService>::clone(&config),
    )
    .await;

    assert_eq!(
        probe
            .run("config-write [plugins.secret-plugin]\\napi_token=\"mine-now\"")
            .await,
        SEC_DENIED
    );
    assert!(config.written().is_empty());
    assert_eq!(config.stored_proxy_document(), proxy_document());
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_the_proxy_config_keeps_other_plugins_blocks_and_secrets_on_disk() {
    let root = tempfile::tempdir().unwrap();
    let servers = root.path().join("servers");
    std::fs::create_dir(&servers).unwrap();
    let path = root.path().join("infrarust.toml");
    let text = format!(
        "servers_dir = {servers:?}\nbind = \"0.0.0.0:25565\"\n\n\
         [web]\nbind = \"127.0.0.1:8080\"\napi_key = \"super-secret-key-value\"\n\n\
         # owned by another plugin\n[plugins.other]\npath = \"/srv/other.wasm\"\npermissions = [\"ban\"]\n\n\
         [plugins.sec-probe]\npermissions = [\"config-write\"]\n"
    );
    std::fs::write(&path, &text).unwrap();
    let config: infrarust_config::ProxyConfig = toml::from_str(&text).unwrap();
    let service = ConfigServiceImpl::new(
        std::sync::Arc::new(DomainRouter::new()),
        path.clone(),
        std::sync::Arc::new(config),
    );
    let probe = enable_sec_with_config(&["config-write"], std::sync::Arc::new(service)).await;

    let dump = probe.run("config-dump").await;
    assert!(dump.starts_with("ok "), "{dump}");
    assert!(
        !dump.contains("/srv/other.wasm") && !dump.contains("super-secret-key-value"),
        "{dump}"
    );
    assert_eq!(
        probe.run("config-edit 0.0.0.0:25565 0.0.0.0:25566").await,
        "ok written"
    );

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("0.0.0.0:25566"), "{on_disk}");
    assert!(
        on_disk.contains("# owned by another plugin") && on_disk.contains("/srv/other.wasm"),
        "{on_disk}"
    );
    assert!(on_disk.contains("super-secret-key-value"), "{on_disk}");
    let written: infrarust_config::ProxyConfig = toml::from_str(&on_disk).unwrap();
    assert_eq!(written.plugins["other"].permissions, ["ban"]);
    assert_eq!(written.plugins["sec-probe"].permissions, ["config-write"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn writing_the_proxy_config_needs_config_write() {
    let probe = enable_sec(&[], "").await;
    let outcome = probe
        .run("config-write [proxy]\\nbind=\\\"0.0.0.0:1\\\"")
        .await;
    assert_eq!(
        outcome, "err ErrorKind::PermissionDenied",
        "write-proxy-config-document without config-write must be refused"
    );
}
