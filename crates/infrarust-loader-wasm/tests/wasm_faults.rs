#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use infrarust_api::command::CommandSource;
use infrarust_api::event::{Event, ResultedEvent};
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::lifecycle::{PostLoginEvent, PreLoginEvent, PreLoginResult};
use infrarust_api::limbo::test_util::RecordingLimboSession;
use infrarust_api::limbo::{HandlerResult, LimboEntryContext, LimboHandler};
use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_api::plugin::Plugin;
use infrarust_api::services::ban_service::BanService;
use infrarust_api::types::{PlayerId, ProtocolVersion, ServerId};
use infrarust_core::event_bus::{EventBusConfig, EventBusImpl};
use infrarust_core::plugin::context::PluginContextImpl;
use infrarust_core::services::command_manager::{CommandManagerImpl, DispatchOutcome};
use infrarust_loader_wasm::WasmPluginLoader;
use tracing::Level;
use tracing::instrument::WithSubscriber;

use support::log_capture::LogCapture;
use support::mock_services::{Gate, GatedBanService, PanickingBanService};
use support::{
    EnvOptions, TestEnv, fresh_loader, load_enabled, loader_from_toml, make_env, make_env_with,
    nil_profile, read_log, script, stage, write_script,
};

const SCRIPTED: &str = "scripted";
const PROMPTLY: Duration = Duration::from_secs(10);
const PATIENT_HANDLER_TIMEOUT: Duration = Duration::from_secs(30);
const QUARANTINE_AFTER_TWO: &str =
    "[plugins.scripted.wasm.recovery]\nmax_restarts = 2\nbackoff_initial = \"30s\"\n";
const PAST_THE_BACKOFF: Duration = Duration::from_secs(31);

struct Enabled {
    _tmp: tempfile::TempDir,
    data: PathBuf,
    env: TestEnv,
    loader: WasmPluginLoader,
    plugin: Box<dyn Plugin>,
}

async fn enable_scripted(script: &str, proxy_toml: &str) -> Enabled {
    let (tmp, plugins_dir) = stage(SCRIPTED);
    write_script(&plugins_dir, SCRIPTED, script);
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions::default().grant(SCRIPTED, "chat-intercept"),
    );
    enable(tmp, plugins_dir, env, SCRIPTED, proxy_toml).await
}

async fn enable_with_bans(
    fixture: &str,
    ban_service: Arc<dyn BanService>,
    proxy_toml: &str,
) -> Enabled {
    let (tmp, plugins_dir) = stage(fixture);
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions {
            ban_service,
            bus_config: EventBusConfig {
                handler_timeout: PATIENT_HANDLER_TIMEOUT,
                ..EventBusConfig::default()
            },
            ..EnvOptions::default()
        }
        .grant(fixture, "ban"),
    );
    enable(tmp, plugins_dir, env, fixture, proxy_toml).await
}

async fn enable(
    tmp: tempfile::TempDir,
    plugins_dir: PathBuf,
    env: TestEnv,
    id: &str,
    proxy_toml: &str,
) -> Enabled {
    let loader = loader_from_toml(proxy_toml);
    loader.discover(&plugins_dir).await.unwrap();
    let plugin = load_enabled(&loader, &env.factory, id).await;
    Enabled {
        _tmp: tmp,
        data: plugins_dir.join(id),
        env,
        loader,
        plugin,
    }
}

fn pre_login() -> PreLoginEvent {
    PreLoginEvent::new(
        nil_profile("Steve"),
        SocketAddr::from(([127, 0, 0, 1], 25565)),
        ProtocolVersion::MINECRAFT_1_21,
        "play.example.com".to_string(),
    )
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
        Some(ServerId::from("lobby")),
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

fn kinds(data: &Path) -> Vec<String> {
    read_log(data)
        .iter()
        .map(|line| line.split(' ').next().unwrap_or_default().to_owned())
        .collect()
}

fn listeners<E: Event>(bus: &EventBusImpl, owner: &str) -> usize {
    bus.listener_owners::<E>()
        .iter()
        .filter(|listener| listener.as_ref() == owner)
        .count()
}

async fn dispatch(env: &TestEnv, line: &str) -> bool {
    tokio::time::timeout(PROMPTLY, dispatch_line(&env.command_manager, line))
        .await
        .expect("a command returns promptly")
}

async fn let_the_backoff_pass() {
    tokio::time::pause();
    tokio::time::advance(PAST_THE_BACKOFF).await;
    tokio::time::resume();
}

fn break_the_log(data: &Path) -> PathBuf {
    let log = data.join(script::LOG_FILE);
    if log.exists() {
        std::fs::remove_file(&log).unwrap();
    }
    std::fs::create_dir(&log).unwrap();
    log
}

fn limbo_handlers(env: &TestEnv, id: &str) -> Vec<Box<dyn LimboHandler>> {
    env.factory
        .create_context(id)
        .as_any()
        .downcast_ref::<PluginContextImpl>()
        .expect("PluginContextImpl")
        .take_limbo_handlers()
}

fn named(handlers: &mut Vec<Box<dyn LimboHandler>>, name: &str) -> Box<dyn LimboHandler> {
    let at = handlers
        .iter()
        .position(|handler| handler.name() == name)
        .unwrap_or_else(|| panic!("handler {name} not registered"));
    handlers.remove(at)
}

fn limbo_session(player_id: u64) -> Arc<RecordingLimboSession> {
    RecordingLimboSession::new(
        PlayerId::new(player_id),
        nil_profile("Tester"),
        LimboEntryContext::InitialConnection {
            target_server: ServerId::from("hub"),
        },
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trapped_handler_leaves_its_event_unchanged_and_a_fresh_instance_handles_the_next_one() {
    let logs = LogCapture::at(Level::INFO);

    async {
        let fx = enable_scripted(
            "on pre-login normal panic\non chat-message normal modify \"alive\"",
            "",
        )
        .await;

        let login = tokio::time::timeout(PROMPTLY, fx.env.event_bus.fire(pre_login()))
            .await
            .unwrap();
        assert!(
            matches!(login.result(), PreLoginResult::Allowed),
            "the trapped handler left the event unchanged"
        );
        let message = tokio::time::timeout(PROMPTLY, fx.env.event_bus.fire(chat()))
            .await
            .unwrap();
        assert!(
            matches!(message.result(), ChatMessageResult::Modify { message } if message == "alive"),
            "a fresh instance handled the next event"
        );
        assert_eq!(
            kinds(&fx.data),
            ["enable", "pre-login", "enable", "chat-message"],
            "the fresh instance ran on_enable again before handling the next event"
        );
        assert_eq!(listeners::<PreLoginEvent>(&fx.env.event_bus, SCRIPTED), 1);
        assert_eq!(
            listeners::<ChatMessageEvent>(&fx.env.event_bus, SCRIPTED),
            1
        );
    }
    .with_subscriber(logs.clone())
    .await;

    let errors = logs.at_level(Level::ERROR);
    assert_eq!(errors.len(), 1, "one error per fault: {errors:?}");
    assert!(
        errors[0].contains("plugin=scripted")
            && errors[0].contains("op=\"handle-event\"")
            && errors[0].contains("trapped"),
        "the error names the plugin, the call and the cause: {errors:?}"
    );
    let recovered = logs.matching("recovered");
    assert_eq!(recovered.len(), 1, "{:?}", logs.lines());
    assert!(
        recovered[0].starts_with("INFO") && recovered[0].contains("generation=2"),
        "{recovered:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_that_trapped_still_dispatches_to_the_recovered_instance() {
    let fx = enable_with_bans(
        "deadline-probe",
        Arc::new(support::mock_services::MockBanService),
        "",
    )
    .await;
    let log = break_the_log(&fx.data);

    assert!(
        dispatch(&fx.env, "ping").await,
        "the host routes the command"
    );
    std::fs::remove_dir(&log).unwrap();
    assert!(dispatch(&fx.env, "ping").await);

    assert_eq!(
        read_log(&fx.data),
        ["command"],
        "the command trapped writing its log, then ran in the recovered instance"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_cut_off_by_max_call_duration_is_recovered() {
    let gate = Gate::new();
    let logs = LogCapture::at(Level::ERROR);

    async {
        let fx = enable_with_bans(
            "slow-handler",
            Arc::new(GatedBanService {
                gate: Arc::clone(&gate),
            }),
            "[plugins.slow-handler.wasm]\nmax_call_duration = \"200ms\"\n",
        )
        .await;

        tokio::time::timeout(PROMPTLY, fx.env.event_bus.fire(post_login()))
            .await
            .expect("max_call_duration cuts off a guest call parked in a host call");
        assert!(dispatch(&fx.env, "ping").await);

        assert_eq!(
            read_log(&fx.data),
            ["command"],
            "the recovered instance ran the next call"
        );
    }
    .with_subscriber(logs.clone())
    .await;

    let errors = logs.lines();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("plugin=slow-handler")
            && errors[0].contains("op=\"handle-event\"")
            && errors[0].contains("max_call_duration"),
        "{errors:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_panic_inside_a_guest_call_is_recovered() {
    let logs = LogCapture::at(Level::ERROR);

    async {
        let fx = enable_with_bans("slow-handler", Arc::new(PanickingBanService), "").await;

        tokio::time::timeout(PROMPTLY, fx.env.event_bus.fire(post_login()))
            .await
            .expect("a panicking host call must not hang the caller");
        assert!(dispatch(&fx.env, "ping").await);

        assert_eq!(read_log(&fx.data), ["command"]);
    }
    .with_subscriber(logs.clone())
    .await;

    let errors = logs.lines();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("plugin=slow-handler")
            && errors[0].contains("ban service panicked on purpose"),
        "{errors:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn repeated_traps_quarantine_the_plugin_until_its_backoff_passes() {
    let logs = LogCapture::at(Level::WARN);

    async {
        let fx = enable_scripted(
            "on pre-login normal panic\ncmd greet record",
            QUARANTINE_AFTER_TWO,
        )
        .await;

        for _ in 0..3 {
            fx.env.event_bus.fire(pre_login()).await;
        }
        assert_eq!(
            kinds(&fx.data),
            [
                "enable",
                "pre-login",
                "enable",
                "pre-login",
                "enable",
                "pre-login"
            ],
            "two restarts fit in the window, the third fault quarantines the plugin"
        );
        assert_eq!(listeners::<PreLoginEvent>(&fx.env.event_bus, SCRIPTED), 0);

        let started = Instant::now();
        assert!(dispatch(&fx.env, "greet").await, "the host still routes it");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(1),
            "a quarantined plugin answers at once, took {elapsed:?}"
        );
        assert_eq!(kinds(&fx.data).len(), 6, "no guest code ran");

        let_the_backoff_pass().await;
        assert!(dispatch(&fx.env, "greet").await);
        assert_eq!(
            kinds(&fx.data)[6..],
            ["enable", "cmd"],
            "after the backoff a fresh instance is enabled and takes the command"
        );
        assert_eq!(listeners::<PreLoginEvent>(&fx.env.event_bus, SCRIPTED), 1);
    }
    .with_subscriber(logs.clone())
    .await;

    let quarantined = logs.matching("quarantined");
    assert_eq!(quarantined.len(), 1, "{:?}", logs.lines());
    assert!(
        quarantined[0].starts_with("WARN")
            && quarantined[0].contains("plugin=scripted")
            && quarantined[0].contains("retry_in=30s"),
        "{quarantined:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_trap_in_the_recovery_on_enable_counts_as_a_failed_restart() {
    let logs = LogCapture::at(Level::INFO);

    async {
        let fx = enable_scripted(
            "on pre-login normal record\ncmd greet record",
            QUARANTINE_AFTER_TWO,
        )
        .await;
        let log = break_the_log(&fx.data);

        fx.env.event_bus.fire(pre_login()).await;
        assert_eq!(
            listeners::<PreLoginEvent>(&fx.env.event_bus, SCRIPTED),
            0,
            "the listeners of the failed restarts are gone too"
        );

        std::fs::remove_dir(&log).unwrap();
        let_the_backoff_pass().await;
        assert!(dispatch(&fx.env, "greet").await);
        assert_eq!(kinds(&fx.data), ["enable", "cmd"]);
        assert_eq!(listeners::<PreLoginEvent>(&fx.env.event_bus, SCRIPTED), 1);
    }
    .with_subscriber(logs.clone())
    .await;

    let errors = logs.at_level(Level::ERROR);
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(errors[0].contains("op=\"handle-event\""), "{errors:?}");
    assert!(
        errors[1..]
            .iter()
            .all(|error| error.contains("op=\"on-enable\"") && error.contains("trapped")),
        "both restarts trapped in on_enable: {errors:?}"
    );
    assert_eq!(logs.matching("quarantined").len(), 1, "{:?}", logs.lines());
    let recovered = logs.matching("recovered");
    assert_eq!(recovered.len(), 1, "{:?}", logs.lines());
    assert!(recovered[0].contains("generation=4"), "{recovered:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hold_owned_by_the_trapped_instance_is_released_with_the_fallback() {
    let (_tmp, plugins_dir) = stage("limbo-handler");
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions::default().grant("limbo-handler", "limbo"),
    );
    let loader = fresh_loader();
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "limbo-handler").await;
    let mut handlers = limbo_handlers(&env, "limbo-handler");
    let gate = named(&mut handlers, "gate");
    let boom = named(&mut handlers, "boom");

    let held = limbo_session(1);
    assert!(matches!(
        gate.on_player_enter(held.as_ref()).await,
        HandlerResult::Hold
    ));
    assert!(held.completions().is_empty());

    let trapped = limbo_session(2);
    assert!(matches!(
        boom.on_player_enter(trapped.as_ref()).await,
        HandlerResult::Deny(_)
    ));
    let completions = held.completions();
    assert!(
        matches!(completions.as_slice(), [HandlerResult::Deny(reason)] if reason.to_plain() == "Limbo handler unavailable"),
        "the player held by the trapped instance is released, got {completions:?}"
    );

    let next = limbo_session(3);
    assert!(
        matches!(
            gate.on_player_enter(next.as_ref()).await,
            HandlerResult::Hold
        ),
        "the recovered instance serves the same handler"
    );
    assert!(next.completions().is_empty());
    assert!(
        limbo_handlers(&env, "limbo-handler").is_empty(),
        "the recovered instance rebinds its handlers instead of registering them again"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn on_disable_after_a_recovery_leaves_no_listener_and_unload_stops_the_actor() {
    let fx = enable_scripted(
        "on pre-login normal panic\non chat-message normal record",
        "",
    )
    .await;
    fx.env.event_bus.fire(pre_login()).await;
    let context = Arc::downgrade(&fx.env.factory.create_context(SCRIPTED));

    tokio::time::timeout(PROMPTLY, fx.plugin.on_disable())
        .await
        .unwrap()
        .expect("on_disable ok");

    assert_eq!(
        kinds(&fx.data),
        ["enable", "pre-login", "enable", "disable"],
        "the recovered instance ran on_disable"
    );
    assert_eq!(listeners::<PreLoginEvent>(&fx.env.event_bus, SCRIPTED), 0);
    assert_eq!(
        listeners::<ChatMessageEvent>(&fx.env.event_bus, SCRIPTED),
        0
    );
    fx.loader.unload(SCRIPTED).await.unwrap();
    assert!(
        context.upgrade().is_none(),
        "the plugin task ended and released the plugin context"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn on_disable_of_a_quarantined_plugin_skips_the_guest_and_unload_stops_the_actor() {
    let fx = enable_scripted(
        "on pre-login normal panic\ncmd greet record",
        "[plugins.scripted.wasm.recovery]\nmax_restarts = 0\nbackoff_initial = \"5m\"\n",
    )
    .await;
    fx.env.event_bus.fire(pre_login()).await;
    let context = Arc::downgrade(&fx.env.factory.create_context(SCRIPTED));

    tokio::time::timeout(PROMPTLY, fx.plugin.on_disable())
        .await
        .unwrap()
        .expect("on_disable of a quarantined plugin is skipped, not an error");

    assert_eq!(kinds(&fx.data), ["enable", "pre-login"]);
    assert_eq!(listeners::<PreLoginEvent>(&fx.env.event_bus, SCRIPTED), 0);
    fx.loader.unload(SCRIPTED).await.unwrap();
    assert!(context.upgrade().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fault_in_the_first_on_enable_fails_the_enable_without_a_restart() {
    for fixture in ["trap-on-purpose", "cpu-spin"] {
        let logs = LogCapture::at(Level::INFO);

        async {
            let (_tmp, plugins_dir) = stage(fixture);
            let env = make_env(plugins_dir.clone());
            let loader = loader_from_toml("[wasm]\ncpu_budget = \"200ms\"\n");
            loader.discover(&plugins_dir).await.unwrap();
            let plugin = loader.load(fixture, &env.factory).await.unwrap();
            let ctx = env.factory.create_context(fixture);

            let enabled = tokio::time::timeout(PROMPTLY, plugin.on_enable(ctx.as_ref()))
                .await
                .expect("the fault is contained");
            assert!(enabled.is_err(), "{fixture}: the enable fails");
        }
        .with_subscriber(logs.clone())
        .await;

        let errors = logs.at_level(Level::ERROR);
        assert_eq!(errors.len(), 1, "{fixture}: {errors:?}");
        assert!(errors[0].contains("op=\"on-enable\""), "{errors:?}");
        assert!(
            logs.matching("recovered").is_empty() && logs.matching("quarantined").is_empty(),
            "{fixture}: a plugin that never enabled is not restarted: {:?}",
            logs.lines()
        );
    }
}

async fn dispatch_line(commands: &CommandManagerImpl, line: &str) -> bool {
    commands.dispatch(CommandSource::Console, line).await == DispatchOutcome::Executed
}
