#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use infrarust_api::command::{CommandContext, CommandHandler, CommandSource, CommandSpec};
use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{BoxFuture, EventPriority};
use infrarust_api::events::lifecycle::PostLoginEvent;
use infrarust_api::events::named::NamedEvent;
use infrarust_api::loader::PluginLoader;
use infrarust_api::permissions::PermissionMap;
use infrarust_api::plugin::Plugin;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::services::command_manager::DispatchOutcome;
use infrarust_loader_wasm::WasmPluginLoader;
use tracing::Level;
use tracing::instrument::WithSubscriber;

use support::log_capture::LogCapture;
use support::{
    EnvOptions, TestEnv, load_enabled, loader_from_toml, make_env_with, read_log, stage,
};

const PROBE: &str = "sem-probe";
const PROMPTLY: Duration = Duration::from_secs(10);

struct Probe {
    _tmp: tempfile::TempDir,
    data: PathBuf,
    env: TestEnv,
    _loader: WasmPluginLoader,
    plugin: Box<dyn Plugin>,
}

impl Probe {
    async fn start(config: &str) -> Self {
        Self::start_with(config, EnvOptions::default(), "", |_| {}).await
    }

    async fn start_with(
        config: &str,
        options: EnvOptions,
        proxy_toml: &str,
        before_load: impl FnOnce(&TestEnv),
    ) -> Self {
        let (tmp, plugins_dir) = stage(PROBE);
        let data = plugins_dir.join(PROBE);
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("probe.txt"), config).unwrap();
        let env = make_env_with(plugins_dir.clone(), options);
        before_load(&env);
        let loader = loader_from_toml(proxy_toml);
        loader.discover(&plugins_dir).await.unwrap();
        let plugin = load_enabled(&loader, &env.factory, PROBE).await;
        Self {
            _tmp: tmp,
            data,
            env,
            _loader: loader,
            plugin,
        }
    }

    fn log(&self) -> Vec<String> {
        read_log(&self.data)
    }

    fn lines_starting(&self, prefix: &str) -> Vec<String> {
        self.log()
            .into_iter()
            .filter(|line| line.starts_with(prefix))
            .collect()
    }

    async fn post_login(&self) {
        let player = support::session_player(
            1,
            support::nil_profile("Steve"),
            767,
            "203.0.113.7:40000".parse().unwrap(),
        );
        tokio::time::timeout(
            PROMPTLY,
            self.env.event_bus.fire(PostLoginEvent::new(player)),
        )
        .await
        .expect("the event finishes promptly");
    }

    async fn dispatch_as(&self, source: CommandSource, line: &str) -> DispatchOutcome {
        tokio::time::timeout(PROMPTLY, self.env.command_manager.dispatch(source, line))
            .await
            .expect("a command returns promptly")
    }

    async fn dispatch(&self, line: &str) -> DispatchOutcome {
        self.dispatch_as(support::console(), line).await
    }

    async fn wait_for_prefix(&self, prefix: &str) {
        let deadline = Instant::now() + PROMPTLY;
        while self.lines_starting(prefix).is_empty() {
            assert!(
                Instant::now() < deadline,
                "never logged a line starting with {prefix:?}: {:?}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn wait_for(&self, line: &str) {
        let deadline = Instant::now() + PROMPTLY;
        while !self.log().iter().any(|seen| seen == line) {
            assert!(
                Instant::now() < deadline,
                "never logged {line:?}: {:?}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn unprivileged() -> CommandSource {
    CommandSource::console(Arc::new(PermissionMap::new()))
}

struct Noop;

impl CommandHandler for Noop {
    fn execute<'a>(&'a self, _ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listener_added_during_dispatch_first_sees_the_next_event() {
    let probe = Probe::start("nest").await;
    probe.post_login().await;
    probe.post_login().await;
    assert_eq!(probe.log(), ["enable", "outer 1", "outer 2", "inner 1"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listener_cancelling_itself_runs_once_and_the_others_keep_running() {
    let probe = Probe::start("selfcancel").await;
    for _ in 0..3 {
        probe.post_login().await;
    }
    assert_eq!(
        probe.log(),
        ["enable", "selfcancel 1", "after 1", "after 2", "after 3"]
    );
}

fn native_cancel_later(bus: &Arc<EventBusImpl>, log: &Arc<Mutex<Vec<String>>>) {
    let dynamic: &dyn EventBus = &**bus;
    let later_log = Arc::clone(log);
    let later = dynamic.subscribe::<PostLoginEvent, _>(EventPriority::LATE, move |_| {
        let mut log = later_log.lock().unwrap();
        let n = log.iter().filter(|line| line.starts_with("later")).count() + 1;
        log.push(format!("later {n}"));
    });
    let slot = Mutex::new(Some(later));
    let weak = Arc::downgrade(bus);
    let canceller_log = Arc::clone(log);
    dynamic.subscribe::<PostLoginEvent, _>(EventPriority::EARLY, move |_| {
        canceller_log.lock().unwrap().push("canceller".to_owned());
        if let (Some(later), Some(bus)) = (slot.lock().unwrap().take(), weak.upgrade()) {
            EventBus::unsubscribe(&*bus, later);
        }
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_later_listener_mid_dispatch_matches_a_native_listener() {
    let bus = Arc::new(EventBusImpl::new());
    let native_log = Arc::new(Mutex::new(Vec::new()));
    native_cancel_later(&bus, &native_log);
    for _ in 0..2 {
        let player = support::session_player(
            1,
            support::nil_profile("Steve"),
            767,
            "203.0.113.7:40000".parse().unwrap(),
        );
        bus.fire(PostLoginEvent::new(player)).await;
    }
    let native = native_log.lock().unwrap().clone();

    let probe = Probe::start("cancel-later").await;
    probe.post_login().await;
    probe.post_login().await;
    let wasm: Vec<String> = probe.log().into_iter().skip(1).collect();
    assert_eq!(
        wasm, native,
        "a native listener cancelled mid-dispatch still gets the event being dispatched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn many_listeners_on_one_event_all_run_in_registration_order() {
    let probe = Probe::start("many 300").await;
    probe.post_login().await;
    let expected: Vec<String> = (0..300).map(|i| format!("many {i}")).collect();
    assert_eq!(probe.lines_starting("many "), expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interval_task_that_cancels_itself_stops_after_its_last_run() {
    let probe = Probe::start("interval a 20 0 3").await;
    probe.wait_for("stop a 3").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let ticks = probe.lines_starting("tick a ");
    assert_eq!(ticks.len(), 3, "{:?}", probe.log());
}

#[tokio::test(flavor = "multi_thread")]
async fn tasks_scheduled_before_disable_never_run_after_it() {
    let probe = Probe::start("delay late 400\ninterval t 50 0 0").await;
    probe.wait_for_prefix("tick t ").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    tokio::time::timeout(PROMPTLY, probe.plugin.on_disable())
        .await
        .expect("disable returns promptly")
        .expect("disable succeeds");
    tokio::time::sleep(Duration::from_millis(600)).await;
    let log = probe.log();
    let disabled = log.iter().position(|line| line == "disable").unwrap();
    let after: Vec<&String> = log[disabled..]
        .iter()
        .filter(|line| line.starts_with("tick") || line.starts_with("fired"))
        .collect();
    assert!(after.is_empty(), "ran after disable: {after:?}");
}

fn tick_times(lines: &[String]) -> Vec<u64> {
    lines
        .iter()
        .filter_map(|line| line.rsplit(' ').next()?.parse().ok())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interval_slower_than_its_period_does_not_starve_the_plugins_other_calls() {
    let probe = Probe::start("interval slow 20 100 0\ncmd ping").await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let started = Instant::now();
    assert_eq!(probe.dispatch("ping").await, DispatchOutcome::Executed);
    let waited = started.elapsed();
    let ticks = probe.lines_starting("tick slow ");
    let times = tick_times(&ticks);
    let gaps: Vec<u64> = times.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert!(
        waited < Duration::from_millis(500),
        "a command waited {waited:?} behind queued runs of a 20 ms interval whose runs take 100 ms \
         ({} runs so far, gaps {gaps:?})",
        ticks.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interval_slower_than_its_period_keeps_at_least_one_period_between_runs_like_native() {
    let probe = Probe::start("interval slow 20 100 0").await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let before = probe.lines_starting("tick slow ").len();
    drop(probe.plugin.on_disable().await);
    let times = tick_times(&probe.lines_starting("tick slow "));
    let gaps: Vec<u64> = times.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert!(
        gaps.iter().all(|gap| *gap >= 100 + 20 - 5),
        "a native repeating task waits one period after each run ends; the WASM runs were {gaps:?} ms \
         apart ({before} runs in 1.2 s)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interval_slower_than_its_period_does_not_flood_the_log_with_expired_runs() {
    let logs = LogCapture::at(Level::WARN);
    async {
        let probe = Probe::start_with(
            "interval slow 20 100 0",
            EnvOptions::default(),
            "[wasm]\nmax_call_duration = \"1s\"\ncpu_budget = \"1s\"\n",
            |_| {},
        )
        .await;
        tokio::time::sleep(Duration::from_secs(3)).await;
        drop(probe.plugin.on_disable().await);
    }
    .with_subscriber(logs.clone())
    .await;
    let expired = logs.matching("its deadline passed while it waited");
    assert!(
        expired.len() <= 5,
        "{} warnings in 3 s for runs of one interval that expired in the queue",
        expired.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_registered_from_a_handler_runs_and_a_command_can_unregister_itself() {
    let probe = Probe::start("nested-commands").await;
    assert_eq!(probe.dispatch("mk").await, DispatchOutcome::Executed);
    assert_eq!(probe.dispatch("made").await, DispatchOutcome::Executed);
    assert_eq!(probe.dispatch("once").await, DispatchOutcome::Executed);
    assert_eq!(probe.dispatch("once").await, DispatchOutcome::Unknown);
    assert_eq!(
        probe.log(),
        [
            "enable",
            "mk ok",
            "made ran",
            "once ran",
            "once unregistered true"
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn registering_a_command_again_applies_the_new_permission_and_handler() {
    let probe = Probe::start("reregister warp warps.admin").await;
    assert_eq!(
        probe.dispatch_as(unprivileged(), "warp").await,
        DispatchOutcome::Denied
    );
    assert_eq!(probe.dispatch("warp").await, DispatchOutcome::Executed);
    assert_eq!(
        probe.log(),
        [
            "rereg warp v1 ok",
            "rereg warp v2 ok",
            "enable",
            "ran-v2 warp"
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_names_and_aliases_follow_the_shared_ownership_rules() {
    let probe = Probe::start_with(
        "cmd spawn\ncmd warp alias sp alias w alias w\ncmd Hub perm hub.use",
        EnvOptions::default(),
        "",
        |env| {
            env.command_manager
                .register_owned(
                    "native",
                    CommandSpec::new("spawn").aliases(["sp"]),
                    Box::new(Noop),
                )
                .unwrap();
        },
    )
    .await;
    assert_eq!(probe.dispatch("w a b").await, DispatchOutcome::Executed);
    assert_eq!(
        probe.dispatch("sem-probe:warp").await,
        DispatchOutcome::Executed
    );
    assert_eq!(probe.dispatch("HUB x").await, DispatchOutcome::Executed);
    assert_eq!(
        probe.dispatch_as(unprivileged(), "hub").await,
        DispatchOutcome::Denied
    );
    assert_eq!(
        probe.log(),
        [
            "reg spawn conflict",
            "reg warp ok name=warp namespaced=sem-probe:warp aliases=w rejected=sp",
            "reg Hub ok name=hub namespaced=sem-probe:hub aliases= rejected=",
            "enable",
            "ran warp label=w args=a,b raw=w a b sender=Console",
            "ran warp label=sem-probe:warp args= raw=sem-probe:warp sender=Console",
            "ran Hub label=HUB args=x raw=HUB x sender=Console",
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_can_be_unregistered_by_any_label_it_answers_to() {
    let probe =
        Probe::start("cmd Warp alias w\nunregister warp\ncmd Home alias h\nunregister h").await;
    assert_eq!(probe.dispatch("warp").await, DispatchOutcome::Unknown);
    assert_eq!(
        probe.dispatch("home").await,
        DispatchOutcome::Unknown,
        "unregistering by an alias removes the command, like on the host: {:?}",
        probe.log()
    );
}

async fn completions(probe: &Probe, inputs: &[&str]) -> Vec<String> {
    for input in inputs {
        let suggestions = tokio::time::timeout(
            PROMPTLY,
            probe.env.command_manager.suggest(support::console(), input),
        )
        .await
        .unwrap();
        assert!(suggestions.is_some(), "{input:?}");
    }
    probe.lines_starting("tab ")
}

#[tokio::test(flavor = "multi_thread")]
async fn tab_completion_hands_the_completer_the_tokens_typed_so_far() {
    let probe = Probe::start("tab warp").await;
    assert_eq!(
        completions(&probe, &["warp a", "warp a ", "warp  a  b", "warp é"]).await,
        [
            "tab warp n=1 args=[a] cursor=1 partial=a",
            "tab warp n=2 args=[a|] cursor=2 partial=",
            "tab warp n=2 args=[a|b] cursor=5 partial=b",
            "tab warp n=1 args=[é] cursor=2 partial=é",
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tab_completion_of_the_first_argument_gets_an_empty_token_after_the_trailing_space() {
    let probe = Probe::start("tab warp").await;
    assert_eq!(
        completions(&probe, &["warp ", "warp a "]).await,
        [
            "tab warp n=1 args=[] cursor=0 partial=",
            "tab warp n=2 args=[a|] cursor=2 partial=",
        ],
        "the docs promise the last token is the one under the cursor, empty after a trailing space; \
         a completer that counts args to know which argument it completes must see one empty token \
         for `warp `, as it sees two for `warp a `"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_with_a_huge_argument_list_reaches_the_guest_intact() {
    let probe = Probe::start("cmd big").await;
    let args: Vec<String> = (0..20_000).map(|i| format!("a{i}")).collect();
    let line = format!("big {}", args.join(" "));
    let started = Instant::now();
    assert_eq!(probe.dispatch(&line).await, DispatchOutcome::Executed);
    let took = started.elapsed();
    let ran = probe.lines_starting("ran big ");
    assert_eq!(ran.len(), 1);
    assert!(ran[0].contains(&format!("args={}", args.join(","))));
    assert!(took < Duration::from_secs(2), "{took:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mixed_case_command_unregistered_then_registered_again_by_a_recovered_instance_is_back() {
    let probe = Probe::start("tools\ncmd Warp").await;
    assert_eq!(probe.dispatch("warp").await, DispatchOutcome::Executed);
    assert_eq!(
        probe.dispatch("unreg Warp").await,
        DispatchOutcome::Executed
    );
    assert_eq!(probe.dispatch("warp").await, DispatchOutcome::Unknown);
    assert_eq!(probe.dispatch("trap").await, DispatchOutcome::Executed);
    probe.wait_for("enable recovered 1").await;
    assert_eq!(
        probe.dispatch("warp").await,
        DispatchOutcome::Executed,
        "the recovered instance registered `Warp` again: {:?}",
        probe.log()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lowercase_command_unregistered_then_registered_again_by_a_recovered_instance_is_back() {
    let probe = Probe::start("tools\ncmd warp").await;
    assert_eq!(
        probe.dispatch("unreg warp").await,
        DispatchOutcome::Executed
    );
    assert_eq!(probe.dispatch("warp").await, DispatchOutcome::Unknown);
    assert_eq!(probe.dispatch("trap").await, DispatchOutcome::Executed);
    probe.wait_for("enable recovered 1").await;
    assert_eq!(probe.dispatch("warp").await, DispatchOutcome::Executed);
}

#[tokio::test(flavor = "multi_thread")]
async fn fire_named_returns_a_timeout_error_when_the_listeners_outlive_the_host_call_timeout() {
    let probe = Probe::start_with(
        "fire ask slow\nnamed-uncancel quick",
        EnvOptions::default(),
        "[wasm]\nhost_call_timeout = \"300ms\"\n",
        |env| {
            let bus: &dyn EventBus = &*env.event_bus;
            bus.subscribe_async::<NamedEvent, _>(EventPriority::NORMAL, |event| {
                let slow = event.name == "slow";
                Box::pin(async move {
                    if slow {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                })
            });
            bus.subscribe::<NamedEvent, _>(EventPriority::EARLY, |event| {
                if event.name == "quick" {
                    event.cancel();
                    event.respond("text/plain", "early");
                }
            });
        },
    )
    .await;
    let started = Instant::now();
    assert_eq!(probe.dispatch("ask").await, DispatchOutcome::Executed);
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        probe.lines_starting("fire slow"),
        ["fire slow timeout after 300ms"]
    );

    let quick = probe
        .env
        .event_bus
        .fire(NamedEvent::new("quick", "text/plain", "ping"))
        .await;
    assert!(!quick.cancelled, "the WASM listener at LATE uncancelled it");
    assert!(
        quick.response.is_none(),
        "the WASM listener cleared the response"
    );
    assert_eq!(
        probe.lines_starting("named quick"),
        ["named quick saw cancelled=true response=early"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delays_and_intervals_fire_on_time_without_drifting() {
    let probe = Probe::start("delay once 300\ninterval acc 50 0 20").await;
    probe.wait_for("stop acc 20").await;
    probe.wait_for_prefix("fired once ").await;
    let fired = tick_times(&probe.lines_starting("fired once "));
    let ticks = tick_times(&probe.lines_starting("tick acc "));
    assert_eq!(ticks.len(), 20);
    assert!(
        (290..=400).contains(&fired[0]),
        "delay fired at {} ms",
        fired[0]
    );
    let last = *ticks.last().unwrap();
    assert!(
        (950..=1200).contains(&last),
        "the 20th run of a 50 ms interval came at {last} ms: {ticks:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_a_plugin_whose_interval_is_slower_than_its_period_returns_promptly() {
    let probe = Probe::start("interval slow 20 100 0").await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let started = Instant::now();
    let disabled = tokio::time::timeout(Duration::from_secs(60), probe.plugin.on_disable()).await;
    let took = started.elapsed();
    let runs = probe.lines_starting("tick slow ").len();
    assert!(disabled.is_ok(), "on_disable did not return within 60 s");
    assert!(
        took < Duration::from_secs(1),
        "on_disable waited {took:?} behind the queued runs of one interval ({runs} runs executed)"
    );
}

fn pre_login(name: &str) -> infrarust_api::events::lifecycle::PreLoginEvent {
    infrarust_api::events::lifecycle::PreLoginEvent::new(
        support::nil_profile(name),
        "203.0.113.7:40000".parse().unwrap(),
        infrarust_api::types::ProtocolVersion::new(767),
        "play.example.com".to_owned(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deny_listener_busy_with_one_slow_login_still_decides_the_logins_behind_it() {
    use infrarust_api::event::ResultedEvent;
    use infrarust_api::events::lifecycle::PreLoginResult;

    let probe = Probe::start_with(
        "whitelist",
        EnvOptions {
            bus_config: infrarust_core::event_bus::EventBusConfig {
                handler_timeout: Duration::from_millis(300),
                ..infrarust_core::event_bus::EventBusConfig::default()
            },
            ..EnvOptions::default()
        },
        "[events]\nhandler_timeout = \"300ms\"\n",
        |_| {},
    )
    .await;
    let bus = Arc::clone(&probe.env.event_bus);
    let slow = tokio::spawn(async move { bus.fire(pre_login("Sleep2000")).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut outcomes = Vec::new();
    for name in ["Intruder", "Griefer", "Friend"] {
        if name == "Griefer" {
            let deadline = Instant::now() + PROMPTLY;
            while probe.lines_starting("enable recovered").is_empty() {
                assert!(
                    Instant::now() < deadline,
                    "the slow call was never replaced: {:?}",
                    probe.log()
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        let event = probe.env.event_bus.fire(pre_login(name)).await;
        outcomes.push(format!(
            "{name}={}",
            match event.result() {
                PreLoginResult::Denied { .. } => "denied",
                PreLoginResult::Allowed => "allowed",
                _ => "other",
            }
        ));
    }
    drop(slow.await);
    assert_eq!(
        outcomes,
        ["Intruder=denied", "Griefer=denied", "Friend=allowed"],
        "while the whitelist plugin runs one slow login, the logins behind it skip the plugin \
         and are let in (plugin log {:?})",
        probe.log()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delay_that_already_fired_is_no_longer_tracked_by_the_host() {
    let logs = LogCapture::at(Level::DEBUG);
    let config: Vec<String> = (0..200).map(|i| format!("delay d{i} 1")).collect();
    let config = config.join("\n");
    async {
        let probe = Probe::start(&config).await;
        let deadline = Instant::now() + PROMPTLY;
        while probe.lines_starting("fired ").len() < 200 {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        probe.plugin.on_disable().await.unwrap();
    }
    .with_subscriber(logs.clone())
    .await;
    let stale = logs.matching("cancel ignored");
    assert!(
        stale.is_empty(),
        "the host still held {} one-shot tasks that had already run, and cancelled each of them at disable",
        stale.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_holding_its_full_quotas_recovers_with_every_registration() {
    let probe = Probe::start_with(
        "tools\ncmd warp\ncmd home\nmany 3\ninterval idle 60000 0 0",
        EnvOptions::default(),
        "[wasm.quotas]\ncommands = 4\nevent_listeners = 3\nscheduled_tasks = 1\n",
        |_| {},
    )
    .await;
    for attempt in 1..=3 {
        assert_eq!(probe.dispatch("trap").await, DispatchOutcome::Executed);
        probe.wait_for(&format!("enable recovered {attempt}")).await;
    }
    assert_eq!(probe.dispatch("warp").await, DispatchOutcome::Executed);
    assert_eq!(probe.dispatch("home").await, DispatchOutcome::Executed);
    probe.post_login().await;
    let deadline = Instant::now() + PROMPTLY;
    while probe.lines_starting("many ").len() < 3 {
        assert!(Instant::now() < deadline, "{:?}", probe.log());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_registration_refused_by_a_quota_in_on_enable_fails_the_enable() {
    use infrarust_api::loader::PluginContextFactory;

    let (_tmp, plugins_dir) = stage(PROBE);
    let data = plugins_dir.join(PROBE);
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        data.join("probe.txt"),
        "cmd one\ncmd two\ncmd three\nmany 3",
    )
    .unwrap();
    let env = make_env_with(plugins_dir.clone(), EnvOptions::default());
    let loader =
        loader_from_toml("[plugins.sem-probe.wasm.quotas]\ncommands = 2\nevent_listeners = 2\n");
    loader.discover(&plugins_dir).await.unwrap();
    let plugin = loader.load(PROBE, &env.factory).await.unwrap();
    let ctx = env.factory.create_context(PROBE);
    let refused = plugin.on_enable(ctx.as_ref()).await.unwrap_err();
    assert!(refused.to_string().contains("limit-exceeded"), "{refused}");
    let log = read_log(&data);
    assert!(
        log.iter().any(|line| line == "reg three limit-exceeded"),
        "a refusal the plugin handles itself does not fail the enable: {log:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_wasm_plugin_cannot_take_a_command_or_alias_another_wasm_plugin_owns() {
    let (tmp, plugins_dir) = stage(PROBE);
    support::add_fixture(&plugins_dir, "scripted-peer", "scripted-peer");
    support::write_script(&plugins_dir, "scripted-peer", "cmd greet record");
    let data = plugins_dir.join(PROBE);
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("probe.txt"), "cmd greet\ncmd hello alias greet").unwrap();
    let env = make_env_with(plugins_dir.clone(), EnvOptions::default());
    let loader = loader_from_toml("");
    loader.discover(&plugins_dir).await.unwrap();
    let _peer = load_enabled(&loader, &env.factory, "scripted-peer").await;
    let _probe = load_enabled(&loader, &env.factory, PROBE).await;
    assert_eq!(
        env.command_manager
            .dispatch(support::console(), "greet x")
            .await,
        DispatchOutcome::Executed
    );
    assert_eq!(
        env.command_manager
            .dispatch(support::console(), "sem-probe:hello")
            .await,
        DispatchOutcome::Executed
    );
    assert_eq!(
        read_log(&data),
        [
            "reg greet conflict",
            "reg hello ok name=hello namespaced=sem-probe:hello aliases= rejected=greet",
            "enable",
            "ran hello label=sem-probe:hello args= raw=sem-probe:hello sender=Console",
        ]
    );
    assert_eq!(
        read_log(&plugins_dir.join("scripted-peer")),
        ["enable", "cmd greet x -"]
    );
    drop(tmp);
}
