#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod fault_lab;
mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::chat::ChatMessageResult;
use infrarust_api::limbo::HandlerResult;
use infrarust_api::loader::PluginLoader;
use tracing::Level;
use tracing::instrument::WithSubscriber;

use fault_lab::faults;
use fault_lab::{LAB, Lab, LabOptions, LabPlugin, PROMPTLY};
use support::log_capture::LogCapture;

const EVERYTHING: &str = "interval 60000\nlisten lab-ping\nchannel fault-lab:chan\nlimbo";

fn options(proxy_toml: &str) -> LabOptions {
    LabOptions {
        proxy_toml: proxy_toml.to_owned(),
        ..LabOptions::default()
    }
}

fn everything() -> LabPlugin {
    LabPlugin::lab(EVERYTHING)
        .grant("limbo")
        .grant("codec-filter")
        .grant("plugin-messaging")
}

#[derive(Debug, PartialEq, Eq)]
struct Registrations {
    listeners: usize,
    tasks: usize,
    commands: Vec<String>,
    limbo_handlers: usize,
    codec_filters: Vec<String>,
    channels: usize,
}

fn registrations(lab: &Lab) -> Registrations {
    Registrations {
        listeners: lab.listeners(LAB),
        tasks: lab.tasks(LAB),
        commands: lab.command_names(LAB),
        limbo_handlers: lab.limbo_handlers(LAB).len(),
        codec_filters: lab.codecs.owned_by(LAB),
        channels: lab.channels(LAB),
    }
}

fn open_fds() -> usize {
    std::fs::read_dir("/proc/self/fd").map_or(0, Iterator::count)
}

async fn recover_once(lab: &Lab, round: u64) {
    let handler = lab.limbo_handler(LAB, faults::LIMBO_HANDLER);
    let held = fault_lab::limbo_session(round);
    assert!(matches!(
        handler.on_player_enter(held.as_ref()).await,
        HandlerResult::Hold
    ));
    lab.dispatch("labreg").await;
    lab.set_faults(LAB, &format!("{EVERYTHING}\ncommand panic"));
    lab.dispatch("lab").await;
    lab.set_faults(LAB, EVERYTHING);
    let completions = held.completions();
    assert!(
        matches!(completions.as_slice(), [HandlerResult::Deny(reason)] if reason.to_plain() == "Limbo handler unavailable"),
        "round {round}: the held player is released exactly once: {completions:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hundreds_of_recoveries_leave_exactly_one_set_of_registrations_and_no_growth() {
    let lab = Lab::start(
        vec![everything()],
        options("[wasm]\nmemory_limit_mb = 16\n\n[wasm.recovery]\nmax_restarts = 1000\n"),
    )
    .await;
    let baseline = registrations(&lab);
    assert_eq!(baseline.tasks, 1);
    assert_eq!(baseline.limbo_handlers, 1);
    assert_eq!(baseline.channels, 1);
    assert_eq!(baseline.codec_filters, [faults::CODEC_FILTER_ID]);

    for round in 0..40 {
        recover_once(&lab, round).await;
    }
    let warm = (fault_lab::alive_tasks(), fault_lab::rss_kib(), open_fds());
    let started = Instant::now();
    let rounds = 400;
    for round in 40..40 + rounds {
        recover_once(&lab, round).await;
    }
    let per_recovery = started.elapsed() / u32::try_from(rounds).unwrap();
    let after = (fault_lab::alive_tasks(), fault_lab::rss_kib(), open_fds());

    assert_eq!(
        registrations(&lab),
        baseline,
        "after {} recoveries the plugin holds exactly what one instance registered",
        rounds + 40
    );
    let enables = fault_lab::count_prefix(&lab.log(LAB), "enable recovered");
    assert_eq!(enables, 440);
    println!(
        "recoveries={rounds} per_recovery={per_recovery:?} tasks {}->{} rss_kib {}->{} fds {}->{}",
        warm.0, after.0, warm.1, after.1, warm.2, after.2
    );
    assert!(
        after.0 <= warm.0 + 4,
        "tokio tasks grew: {warm:?} -> {after:?}"
    );
    assert!(
        after.2 <= warm.2 + 4,
        "open fds grew: {warm:?} -> {after:?}"
    );
    assert!(
        after.1 < warm.1 + 64 * 1024,
        "RSS grew by {} KiB over {rounds} recoveries",
        after.1.saturating_sub(warm.1)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hundreds_of_failed_restarts_leave_nothing_behind() {
    let lab = Lab::start(
        vec![everything()],
        options("[wasm]\nmemory_limit_mb = 16\n\n[wasm.recovery]\nmax_restarts = 300\nwindow = \"1h\"\nbackoff_initial = \"5m\"\n"),
    )
    .await;
    let baseline = registrations(&lab);
    let before = (fault_lab::alive_tasks(), fault_lab::rss_kib(), open_fds());
    lab.set_faults(LAB, &format!("{EVERYTHING}\ncommand panic\nenable panic"));
    let started = Instant::now();
    tokio::time::timeout(
        Duration::from_secs(120),
        lab.commands.dispatch(support::console(), "lab"),
    )
    .await
    .expect("300 failed restarts end in a quarantine");
    let took = started.elapsed();
    let after = (fault_lab::alive_tasks(), fault_lab::rss_kib(), open_fds());
    let tried = fault_lab::count_prefix(&lab.log(LAB), "enable recovered");
    println!(
        "failed restarts={tried} in {took:?} tasks {}->{} rss_kib {}->{} fds {}->{}",
        before.0, after.0, before.1, after.1, before.2, after.2
    );
    assert_eq!(tried, 300);
    assert_eq!(lab.tasks(LAB), 0, "no failed instance keeps a task");
    assert_eq!(
        lab.listeners(LAB),
        fault_lab::ACCESS_GUARDS,
        "no failed instance keeps a listener; only the guards that deny its access events remain"
    );
    assert_eq!(registrations(&lab).commands, baseline.commands);
    assert!(
        after.0 <= before.0 + 4,
        "tokio tasks grew: {before:?} -> {after:?}"
    );
    assert!(
        after.2 <= before.2 + 4,
        "open fds grew: {before:?} -> {after:?}"
    );
    assert!(
        after.1 < before.1 + 64 * 1024,
        "RSS grew by {} KiB over {tried} failed restarts",
        after.1.saturating_sub(before.1)
    );
}

const STUCK: &str = "[wasm]\ncpu_budget = \"200ms\"\nmax_call_duration = \"1s\"\n\n[wasm.recovery]\nmax_restarts = 4\nbackoff_initial = \"5m\"\n";

async fn start_a_recovery_loop(lab: &Arc<Lab>) -> tokio::task::JoinHandle<Duration> {
    lab.set_faults(LAB, "limbo\ncommand panic\nenable sleep");
    let looping = Arc::clone(lab);
    let handle = tokio::spawn(async move { looping.dispatch("lab").await });
    lab.wait_for("the first failed restart to start", || {
        fault_lab::count_prefix(&lab.log(LAB), "enable recovered") >= 1
    })
    .await;
    handle
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_call_is_answered_by_its_deadline_while_the_actor_is_stuck_in_a_recovery_loop() {
    let lab =
        Arc::new(Lab::start(vec![LabPlugin::lab("limbo").grant("limbo")], options(STUCK)).await);
    let looping = start_a_recovery_loop(&lab).await;

    let handler = lab.limbo_handler(LAB, faults::LIMBO_HANDLER);
    let session = fault_lab::limbo_session(1);
    let completing = async {
        let started = Instant::now();
        let suggestions = lab.complete("lab ").await;
        (suggestions, started.elapsed())
    };
    let entering = async {
        let started = Instant::now();
        let entered = handler.on_player_enter(session.as_ref()).await;
        (entered, started.elapsed())
    };
    let ((suggestions, completion_wait), (entered, limbo_wait)) =
        tokio::join!(completing, entering);
    let first_caller = looping.await.unwrap();
    println!(
        "faulting command answered after {first_caller:?}, tab completion after {completion_wait:?}, limbo entry after {limbo_wait:?}"
    );

    assert!(suggestions.is_empty());
    assert!(matches!(entered, HandlerResult::Deny(_)));
    assert!(
        completion_wait < Duration::from_millis(1500),
        "a tab completion whose 1s deadline passed while queued waited {completion_wait:?}"
    );
    assert!(
        limbo_wait < Duration::from_millis(1500),
        "a limbo entry whose 1s deadline passed while queued waited {limbo_wait:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unloading_a_plugin_stuck_in_a_recovery_loop_returns_within_one_call_limit() {
    let lab =
        Arc::new(Lab::start(vec![LabPlugin::lab("limbo").grant("limbo")], options(STUCK)).await);
    let looping = start_a_recovery_loop(&lab).await;

    let started = Instant::now();
    tokio::time::timeout(PROMPTLY, lab.loader.unload(LAB))
        .await
        .expect("unload returns")
        .unwrap();
    let unload_took = started.elapsed();
    let _ = looping.await;
    let tried = fault_lab::count_prefix(&lab.log(LAB), "enable recovered");
    println!("unload took {unload_took:?}; restarts tried: {tried}");
    assert!(
        unload_took < Duration::from_millis(1500),
        "unload waited {unload_took:?} for a recovery loop of {tried} restarts"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_a_plugin_stuck_in_a_recovery_loop_returns_within_one_call_limit() {
    let lab =
        Arc::new(Lab::start(vec![LabPlugin::lab("limbo").grant("limbo")], options(STUCK)).await);
    let looping = start_a_recovery_loop(&lab).await;

    let plugin = lab.take(LAB);
    let started = Instant::now();
    let disabled = tokio::time::timeout(PROMPTLY, plugin.on_disable())
        .await
        .expect("on_disable returns");
    let took = started.elapsed();
    let _ = looping.await;
    println!("on_disable took {took:?}: {disabled:?}");
    assert!(
        took < Duration::from_millis(1500),
        "on_disable waited {took:?} for the recovery loop"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interval_run_past_its_deadline_is_cut_as_a_fault_before_the_next_run_starts() {
    let logs = LogCapture::at(Level::ERROR);
    async {
        let lab = Lab::start(
            vec![LabPlugin::lab("interval 50\ntask sleep")],
            options(
                "[wasm]\nmax_call_duration = \"500ms\"\n\n[wasm.recovery]\nmax_restarts = 100\n",
            ),
        )
        .await;
        lab.wait_for("three runs cut at their deadline", || {
            logs.matching("ran past its deadline").len() >= 3
        })
        .await;
        let runs = fault_lab::count(&lab.log(LAB), "task");
        let cuts = logs.matching("ran past its deadline");
        assert!(
            cuts.iter()
                .all(|line| line.contains("op=\"on-scheduled-task\"")),
            "{cuts:?}"
        );
        assert!(
            runs <= cuts.len() + 1,
            "a run started before the one ahead of it was cut: {runs} runs for {} cuts",
            cuts.len()
        );
        lab.wait_for("each cut to replace the instance", || {
            fault_lab::count_prefix(&lab.log(LAB), "enable recovered") >= cuts.len()
        })
        .await;
    }
    .with_subscriber(logs.clone())
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_quarantined_plugin_comes_back_on_its_own_when_its_backoff_passes() {
    let lab = Lab::start(
        vec![LabPlugin::lab("listen lab-ping")],
        options("[wasm.recovery]\nmax_restarts = 0\nbackoff_initial = \"300ms\"\n"),
    )
    .await;
    let listening = lab.listeners(LAB);
    lab.set_faults(LAB, "listen lab-ping\ncommand panic");
    lab.dispatch("lab").await;
    assert_eq!(lab.listeners(LAB), fault_lab::ACCESS_GUARDS, "quarantined");
    let quarantined_at = Instant::now();

    lab.wait_for("the retry after the backoff, without any call", || {
        lab.listeners(LAB) == listening
    })
    .await;
    let back_after = quarantined_at.elapsed();
    assert!(back_after >= Duration::from_millis(250), "{back_after:?}");
    assert_eq!(
        fault_lab::count_prefix(&lab.log(LAB), "enable recovered 1"),
        1,
        "{:?}",
        lab.log(LAB)
    );
}

fn retry_ins(logs: &LogCapture) -> Vec<String> {
    logs.matching("quarantined")
        .iter()
        .filter_map(|line| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("retry_in="))
                .map(str::to_owned)
        })
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn consecutive_quarantines_double_the_backoff_up_to_backoff_max() {
    let logs = LogCapture::at(Level::WARN);
    async {
        let lab = Lab::start(
            vec![LabPlugin::lab("")],
            options("[wasm.recovery]\nmax_restarts = 0\nbackoff_initial = \"1s\"\nbackoff_max = \"4s\"\n"),
        )
        .await;
        lab.set_faults(LAB, "command panic\nenable panic");
        lab.dispatch("lab").await;
        tokio::time::pause();
        tokio::time::sleep(Duration::from_secs(12)).await;
        tokio::time::resume();
    }
    .with_subscriber(logs.clone())
    .await;
    assert_eq!(
        retry_ins(&logs)[..5],
        ["1s", "2s", "4s", "4s", "4s"],
        "{:?}",
        logs.lines()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_long_healthy_period_resets_the_backoff_even_without_a_restart_budget() {
    let logs = LogCapture::at(Level::WARN);
    async {
        let lab = Lab::start(
            vec![LabPlugin::lab("")],
            options("[wasm.recovery]\nmax_restarts = 0\nwindow = \"5m\"\nbackoff_initial = \"1s\"\nbackoff_max = \"5m\"\n"),
        )
        .await;
        for _ in 0..4 {
            lab.set_faults(LAB, "command panic");
            lab.dispatch("lab").await;
            lab.set_faults(LAB, "");
            tokio::time::pause();
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
            tokio::time::resume();
            lab.dispatch("lab").await;
            assert!(lab.log(LAB).iter().any(|line| line == "command "), "healthy again");
        }
    }
    .with_subscriber(logs.clone())
    .await;
    assert_eq!(
        retry_ins(&logs),
        ["1s", "1s", "1s", "1s"],
        "four faults six hours apart are not quarantines in a row: {:?}",
        logs.lines()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn faults_further_apart_than_the_window_never_quarantine() {
    let logs = LogCapture::at(Level::WARN);
    async {
        let lab = Lab::start(
            vec![LabPlugin::lab("")],
            options("[wasm.recovery]\nmax_restarts = 1\nwindow = \"10s\"\n"),
        )
        .await;
        for _ in 0..5 {
            lab.set_faults(LAB, "command panic");
            lab.dispatch("lab").await;
            lab.set_faults(LAB, "");
            tokio::time::pause();
            tokio::time::sleep(Duration::from_secs(11)).await;
            tokio::time::resume();
        }
        assert_eq!(
            fault_lab::count_prefix(&lab.log(LAB), "enable recovered"),
            5
        );
    }
    .with_subscriber(logs.clone())
    .await;
    assert!(
        logs.matching("quarantined").is_empty(),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn events_queued_for_a_replaced_instance_never_reach_the_fresh_instance() {
    let lab = Arc::new(
        Lab::start(
            vec![LabPlugin::lab("")],
            options("[wasm]\ncpu_budget = \"300ms\"\n"),
        )
        .await,
    );
    lab.set_faults(LAB, "event:chat-message spin");
    let mut fired = Vec::new();
    for _ in 0..5 {
        let lab = Arc::clone(&lab);
        fired.push(tokio::spawn(async move {
            let event = lab.event_bus.fire(fault_lab::chat()).await;
            matches!(event.result(), ChatMessageResult::Allow)
        }));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    for event in fired {
        assert!(
            tokio::time::timeout(PROMPTLY, event)
                .await
                .unwrap()
                .unwrap(),
            "every event keeps the result it had"
        );
    }
    let log = lab.log(LAB);
    assert_eq!(
        fault_lab::count(&log, "event:chat-message"),
        1,
        "only the first event reached a guest: {log:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-15: fault cause loses the guest panic message"]
async fn the_recovered_instance_is_told_each_attempt_and_the_cause_of_the_fault() {
    let lab = Lab::start(
        vec![LabPlugin::lab("")],
        options("[wasm]\ncpu_budget = \"200ms\"\nmax_call_duration = \"1s\"\n"),
    )
    .await;
    let mut causes = Vec::new();
    for mode in ["panic", "sleep", "spin", "grow"] {
        lab.set_faults(LAB, &format!("command {mode}"));
        lab.dispatch("lab").await;
        let line = lab
            .log(LAB)
            .into_iter()
            .rfind(|line| line.starts_with("enable recovered"))
            .unwrap();
        causes.push(line);
    }
    println!("{causes:#?}");
    assert!(causes[0].starts_with("enable recovered 1 "), "{causes:?}");
    assert!(
        causes[1].starts_with("enable recovered 2 ") && causes[1].contains("max_call_duration"),
        "{causes:?}"
    );
    assert!(
        causes[2].starts_with("enable recovered 3 ") && causes[2].contains("interrupt"),
        "{causes:?}"
    );
    assert!(causes[3].starts_with("enable recovered 4 "), "{causes:?}");
    assert!(
        causes[0].contains("fault-lab: command panics on purpose"),
        "the cause carries the guest's panic message: {:?}",
        causes[0]
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-15: fault cause loses the guest panic message"]
async fn the_fault_error_carries_the_guest_panic_message() {
    let logs = LogCapture::at(Level::ERROR);
    async {
        let lab = Lab::start(vec![LabPlugin::lab("command panic")], options("")).await;
        lab.dispatch("lab").await;
    }
    .with_subscriber(logs.clone())
    .await;
    let failed = logs.matching("wasm plugin instance failed");
    assert_eq!(failed.len(), 1, "{:?}", logs.lines());
    println!("{}", failed[0]);
    assert!(
        failed[0].contains("fault-lab: command panics on purpose"),
        "an operator can see why the plugin panicked: {:?}",
        failed[0]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unloading_during_a_long_call_waits_for_that_call_only_and_runs_nothing_queued_behind_it() {
    let lab = Arc::new(
        Lab::start(
            vec![LabPlugin::lab("command linger")],
            options("[wasm]\nmax_call_duration = \"5s\"\n"),
        )
        .await,
    );
    let mut queued = Vec::new();
    for _ in 0..5 {
        let lab = Arc::clone(&lab);
        queued.push(tokio::spawn(async move { lab.dispatch("lab").await }));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let started = Instant::now();
    lab.loader.unload(LAB).await.unwrap();
    let took = started.elapsed();
    for call in queued {
        let _ = tokio::time::timeout(PROMPTLY, call).await.unwrap();
    }
    let ran = fault_lab::count(&lab.log(LAB), "command ");
    println!("unload took {took:?}; commands completed: {ran}");
    assert!(took < Duration::from_millis(600), "{took:?}");
    assert_eq!(ran, 1, "only the running call finished: {:?}", lab.log(LAB));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_task_flood_into_a_full_queue_stays_bounded() {
    let lab = Lab::start(
        vec![LabPlugin::lab("interval 1\ntask linger")],
        options("[wasm]\nqueue_capacity = 16\n"),
    )
    .await;
    let started = fault_lab::alive_tasks();
    let mut peak = 0;
    let until = Instant::now() + Duration::from_secs(4);
    while Instant::now() < until {
        peak = peak.max(fault_lab::alive_tasks());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let ran = fault_lab::count(&lab.log(LAB), "task");
    println!("alive tokio tasks {started} -> peak {peak}; tasks run {ran}");
    assert!(
        peak <= started + 16 + 8,
        "waiting callers are bounded by the queue: {started} -> {peak}"
    );
    assert!(ran <= 12, "a lingering task runs about every 400 ms: {ran}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_write_that_returned_before_a_trap_is_on_disk() {
    let lab = Lab::start(
        vec![LabPlugin::lab("command panic")],
        options("[wasm.recovery]\nmax_restarts = 1000\n"),
    )
    .await;
    let rounds = 300;
    for _ in 0..rounds {
        lab.dispatch("lab").await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let log = lab.log(LAB);
    let written = fault_lab::count(&log, "command");
    let enables = fault_lab::count_prefix(&log, "enable recovered");
    println!("command lines {written} of {rounds}; enable lines {enables}");
    assert_eq!(enables, rounds, "every recovery's own log line is there");
    assert_eq!(
        written, rounds,
        "every line the guest wrote before it trapped is in the file"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_write_a_scheduled_task_made_before_it_trapped_is_on_disk() {
    let logs = LogCapture::at(Level::ERROR);
    let (written, faults, log) = async {
        let lab = Lab::start(
            vec![LabPlugin::lab("interval 20\ntask panic")],
            options("[wasm.recovery]\nmax_restarts = 1000\n"),
        )
        .await;
        tokio::time::sleep(Duration::from_secs(4)).await;
        lab.set_faults(LAB, "");
        tokio::time::sleep(Duration::from_millis(300)).await;
        let log = lab.log(LAB);
        (
            fault_lab::count(&log, "task"),
            fault_lab::count_prefix(&log, "enable recovered"),
            log,
        )
    }
    .with_subscriber(logs.clone())
    .await;
    let errors = logs.matching("op=\"on-scheduled-task\"").len();
    println!("task lines {written}; recoveries {faults}; task faults logged {errors}");
    let lost: Vec<&String> = log
        .windows(2)
        .filter(|pair| pair[0] == "enable" && pair[1].starts_with("enable recovered"))
        .map(|pair| &pair[1])
        .collect();
    assert!(
        lost.is_empty(),
        "{} recoveries follow an instance whose only line is `enable`: the `task` line it wrote before trapping is gone",
        lost.len()
    );
    assert!(
        written >= errors,
        "task lines {written} < task faults {errors}"
    );
}
