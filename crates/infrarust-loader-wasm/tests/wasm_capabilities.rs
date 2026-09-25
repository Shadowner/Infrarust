#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::lifecycle::PostLoginEvent;
use infrarust_api::loader::PluginLoader;
use infrarust_api::types::{ProtocolVersion, ServerId};
use infrarust_core::services::command_manager::{CommandManagerImpl, DispatchOutcome};
use infrarust_loader_wasm::WasmPluginLoader;
use tracing::Level;
use tracing::instrument::WithSubscriber;

use support::log_capture::LogCapture;
use support::mock_services::{CountingPlayerRegistry, MapConfigService};
use support::{
    EnvOptions, TestEnv, fresh_loader, load_enabled, loader_from_toml, make_env_with, nil_profile,
    read_log, stage, write_script,
};

const STRICT: &str = "strict_capabilities = true";

fn strict_loader(plugin_id: &str) -> WasmPluginLoader {
    loader_from_toml(&format!("[plugins.{plugin_id}]\n{STRICT}\n"))
}

fn ban_outcome(plugins_dir: &Path) -> String {
    std::fs::read_to_string(plugins_dir.join("capability-denied").join("ban.txt"))
        .expect("the guest recorded its ban-service answer")
}

fn host_caller_env(plugins_dir: &Path, options: EnvOptions) -> TestEnv {
    let values = HashMap::from([("greeting".to_string(), "hello-wasm".to_string())]);
    make_env_with(
        plugins_dir.to_path_buf(),
        EnvOptions {
            player_registry: Arc::new(CountingPlayerRegistry { count: 7 }),
            config_service: Arc::new(MapConfigService { values }),
            ..options
        },
    )
}

fn post_login() -> PostLoginEvent {
    PostLoginEvent::new(support::session_player(
        1,
        nil_profile("Steve"),
        ProtocolVersion::MINECRAFT_1_21.raw(),
        "127.0.0.1:40000".parse().unwrap(),
    ))
}

fn refusal_report<'a>(lines: &'a [String], plugin: &str, interface: &str) -> Vec<&'a String> {
    lines
        .iter()
        .filter(|line| {
            line.contains("calls will be refused")
                && line.contains(plugin)
                && line.contains(interface)
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ungranted_import_loads_and_its_calls_are_refused() {
    let (_tmp, plugins_dir) = stage("capability-denied");
    let loader = fresh_loader();
    let env = make_env_with(plugins_dir.clone(), EnvOptions::default());
    let logs = LogCapture::at(Level::WARN);

    async {
        loader.discover(&plugins_dir).await.unwrap();
        let _plugin = load_enabled(&loader, &env.factory, "capability-denied").await;
    }
    .with_subscriber(logs.clone())
    .await;

    assert_eq!(
        ban_outcome(&plugins_dir),
        "operation-failed: missing capability: ban",
        "the call reached the host and was refused there"
    );
    let lines = logs.lines();
    let report = refusal_report(&lines, "capability-denied", "ban-service");
    assert_eq!(report.len(), 1, "one load-time report line: {lines:?}");
    assert!(report[0].contains("`ban`"), "{report:?}");
    assert!(
        refusal_report(&lines, "capability-denied", "limbo").is_empty(),
        "the limbo resource types every guest imports are not a gated call: {lines:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn strict_capabilities_refuses_an_ungranted_import() {
    let (_tmp, plugins_dir) = stage("capability-denied");
    let loader = strict_loader("capability-denied");
    let env = make_env_with(plugins_dir.clone(), EnvOptions::default());
    loader.discover(&plugins_dir).await.unwrap();

    let err = loader
        .load("capability-denied", &env.factory)
        .await
        .err()
        .expect("strict_capabilities refuses a plugin whose calls would be refused");
    let message = err.to_string();
    for needle in [
        "capability-denied",
        "infrarust:plugin/ban-service",
        "`ban`",
        "strict_capabilities",
    ] {
        assert!(
            message.contains(needle),
            "{needle:?} missing from {message}"
        );
    }
    assert!(
        !plugins_dir
            .join("capability-denied")
            .join("ban.txt")
            .exists(),
        "the refused plugin never ran"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn strict_capabilities_loads_a_plugin_granted_what_it_imports() {
    let (_tmp, plugins_dir) = stage("capability-denied");
    let loader = strict_loader("capability-denied");
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions::default().grant("capability-denied", "ban"),
    );
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "capability-denied").await;

    assert_eq!(ban_outcome(&plugins_dir), "ok: false");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_denied_baseline_capability_refuses_its_calls_at_call_time() {
    let (_tmp, plugins_dir) = stage("host-caller");
    let loader = fresh_loader();
    let env = host_caller_env(
        &plugins_dir,
        EnvOptions::default().deny("host-caller", "config-read"),
    );
    let logs = LogCapture::at(Level::WARN);

    async {
        loader.discover(&plugins_dir).await.unwrap();
        let _plugin = load_enabled(&loader, &env.factory, "host-caller").await;
    }
    .with_subscriber(logs.clone())
    .await;

    let dir = plugins_dir.join("host-caller");
    assert_eq!(
        std::fs::read_to_string(dir.join("count.txt")).expect("count.txt"),
        "7",
        "player-read is still granted"
    );
    assert!(
        !dir.join("greeting.txt").exists(),
        "config-read is denied, so the configured greeting reads as absent"
    );
    let lines = logs.lines();
    let report = refusal_report(&lines, "host-caller", "config-service");
    assert_eq!(report.len(), 1, "{lines:?}");
    assert!(report[0].contains("`config-read`"), "{report:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn strict_capabilities_refuses_a_denied_baseline_import() {
    let (_tmp, plugins_dir) = stage("host-caller");
    let loader = strict_loader("host-caller");
    let env = host_caller_env(
        &plugins_dir,
        EnvOptions::default().deny("host-caller", "config-read"),
    );
    loader.discover(&plugins_dir).await.unwrap();

    let message = loader
        .load("host-caller", &env.factory)
        .await
        .err()
        .expect("a denied baseline import is refused in strict mode")
        .to_string();
    assert!(
        message.contains("infrarust:plugin/config-service") && message.contains("`config-read`"),
        "{message}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_denied_events_and_commands_still_runs() {
    let (_tmp, plugins_dir) = stage("scripted");
    write_script(
        &plugins_dir,
        "scripted",
        "on post-login normal record\ncmd probe record\n",
    );
    let loader = fresh_loader();
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions::default()
            .deny("scripted", "event-bus")
            .deny("scripted", "command"),
    );
    let logs = LogCapture::at(Level::WARN);

    let found = async {
        loader.discover(&plugins_dir).await.unwrap();
        let _plugin = load_enabled(&loader, &env.factory, "scripted").await;
        env.event_bus.fire(post_login()).await;
        dispatch_line(&env.command_manager, "probe").await
    }
    .with_subscriber(logs.clone())
    .await;

    assert!(!found, "the command was never registered with the host");
    assert_eq!(
        read_log(&plugins_dir.join("scripted")),
        ["enable"],
        "on_enable ran to the end and no event reached the guest"
    );
    let lines = logs.lines();
    for (interface, capability) in [
        ("event-bus", "`event-bus`"),
        ("command-manager", "`command`"),
    ] {
        let report = refusal_report(&lines, "scripted", interface);
        assert_eq!(report.len(), 1, "{interface}: {lines:?}");
        assert!(report[0].contains(capability), "{report:?}");
    }
}

async fn dispatch_line(commands: &CommandManagerImpl, line: &str) -> bool {
    commands.dispatch(support::console(), line).await == DispatchOutcome::Executed
}

fn chat() -> ChatMessageEvent {
    ChatMessageEvent::new(
        support::session_player(
            1,
            nil_profile("Steve"),
            ProtocolVersion::MINECRAFT_1_21.raw(),
            "127.0.0.1:40000".parse().unwrap(),
        ),
        "hello".to_string(),
        false,
        Some(ServerId::new("lobby")),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chat_subscription_without_chat_intercept_is_refused() {
    let (_tmp, plugins_dir) = stage("scripted");
    write_script(
        &plugins_dir,
        "scripted",
        "on chat-message normal modify \"rewritten\"\n",
    );
    let loader = fresh_loader();
    let env = make_env_with(plugins_dir.clone(), EnvOptions::default());
    let logs = LogCapture::at(Level::WARN);

    let event = async {
        loader.discover(&plugins_dir).await.unwrap();
        let _plugin = load_enabled(&loader, &env.factory, "scripted").await;
        env.event_bus.fire(chat()).await
    }
    .with_subscriber(logs.clone())
    .await;

    assert!(
        matches!(event.result(), ChatMessageResult::Allow),
        "a plugin without chat-intercept changed a chat message"
    );
    assert_eq!(
        read_log(&plugins_dir.join("scripted")),
        ["enable"],
        "the chat message reached the guest"
    );
    let refused = logs.matching("missing capability `chat-intercept`");
    assert_eq!(refused.len(), 1, "{:?}", logs.lines());
    assert!(
        refused[0].contains("event-bus.subscribe(chat-message)"),
        "{refused:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_intercept_lets_a_wasm_plugin_rewrite_chat() {
    let (_tmp, plugins_dir) = stage("scripted");
    write_script(
        &plugins_dir,
        "scripted",
        "on chat-message normal modify \"rewritten\"\n",
    );
    let loader = fresh_loader();
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions::default().grant("scripted", "chat-intercept"),
    );
    let logs = LogCapture::at(Level::WARN);

    let event = async {
        loader.discover(&plugins_dir).await.unwrap();
        let _plugin = load_enabled(&loader, &env.factory, "scripted").await;
        env.event_bus.fire(chat()).await
    }
    .with_subscriber(logs.clone())
    .await;

    assert_eq!(
        event.result(),
        &ChatMessageResult::Modify {
            message: "rewritten".to_string()
        }
    );
    assert_eq!(read_log(&plugins_dir.join("scripted")).len(), 2);
    assert!(
        logs.matching("chat-intercept").is_empty(),
        "{:?}",
        logs.lines()
    );
}
