#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod fault_lab;
mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use infrarust_api::event::ResultedEvent;
use infrarust_api::events::chat::ChatMessageResult;
use infrarust_api::types::{ProtocolVersion, RawPacket};
use infrarust_core::filter::codec_chain::build_codec_chains;
use tracing::Level;
use tracing::instrument::WithSubscriber;

use fault_lab::faults::{self, Mode};
use fault_lab::{LAB, Lab, LabOptions, LabPlugin, PEER, PROMPTLY};
use support::log_capture::LogCapture;

const HEALTHY: &str = "scripted";
const HEALTHY_SCRIPT: &str = "on chat-message late modify \"alive\"\ncmd greet record";

fn percentile(samples: &mut [Duration], at: f64) -> Duration {
    samples.sort();
    let index = ((samples.len() as f64 - 1.0) * at).round() as usize;
    samples[index]
}

async fn command_latencies(lab: &Lab, rounds: usize) -> Vec<Duration> {
    let mut samples = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        samples.push(lab.dispatch("greet").await);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    samples
}

async fn fire_chats(lab: &Arc<Lab>, senders: usize, each: usize) -> (usize, Vec<Duration>) {
    let mut handles = Vec::new();
    for _ in 0..senders {
        let lab = Arc::clone(lab);
        handles.push(tokio::spawn(async move {
            let mut alive = 0;
            let mut latencies = Vec::with_capacity(each);
            for _ in 0..each {
                let started = Instant::now();
                let event = lab.event_bus.fire(fault_lab::chat()).await;
                latencies.push(started.elapsed());
                if matches!(event.result(), ChatMessageResult::Modify { message } if message == "alive")
                {
                    alive += 1;
                }
            }
            (alive, latencies)
        }));
    }
    let mut alive = 0;
    let mut latencies = Vec::new();
    for handle in handles {
        let (ok, mut samples) = tokio::time::timeout(Duration::from_secs(120), handle)
            .await
            .expect("the storm ends")
            .unwrap();
        alive += ok;
        latencies.append(&mut samples);
    }
    (alive, latencies)
}

fn storm_options(faulty_budget: &str) -> LabOptions {
    LabOptions {
        proxy_toml: format!(
            "[wasm]\ncpu_budget = \"{faulty_budget}\"\nmemory_limit_mb = 16\n\n[wasm.recovery]\nmax_restarts = 1000\n"
        ),
        extra: vec![("scripted", HEALTHY, HEALTHY_SCRIPT.to_owned())],
        ..LabOptions::default()
    }
}

async fn storm(mode: Mode, faulty_budget: &str) {
    let logs = LogCapture::at(Level::WARN);
    async {
        let lab = Arc::new(
            Lab::start(
                vec![LabPlugin::lab(""), LabPlugin::peer("")],
                storm_options(faulty_budget),
            )
            .await,
        );
        lab.enable(HEALTHY).await;

        let mut quiet_commands = command_latencies(&lab, 40).await;
        let (alive, mut quiet_events) = fire_chats(&lab, 4, 10).await;
        assert_eq!(alive, 40);

        let faults = format!("event:chat-message {}", mode.as_str());
        lab.set_faults(LAB, &faults);
        lab.set_faults(PEER, &format!("prefix peer\n{faults}"));
        let stormer = Arc::clone(&lab);
        let storm = tokio::spawn(async move { fire_chats(&stormer, 4, 15).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let mut storm_commands = command_latencies(&lab, 40).await;
        let (alive, mut storm_events) = storm.await.unwrap();

        let recoveries = fault_lab::count_prefix(&lab.log(LAB), "enable recovered")
            + fault_lab::count_prefix(&lab.log(PEER), "enable recovered");
        let healthy_chats = support::read_log(&lab.data(HEALTHY))
            .iter()
            .filter(|line| line.starts_with("chat-message"))
            .count();
        println!(
            "{mode:?} storm: recoveries={recoveries} healthy command p50 {:?}/{:?} p99 {:?}/{:?} (quiet/storm); event p50 {:?}/{:?} p99 {:?}/{:?}",
            percentile(&mut quiet_commands, 0.5),
            percentile(&mut storm_commands, 0.5),
            percentile(&mut quiet_commands, 0.99),
            percentile(&mut storm_commands, 0.99),
            percentile(&mut quiet_events, 0.5),
            percentile(&mut storm_events, 0.5),
            percentile(&mut quiet_events, 0.99),
            percentile(&mut storm_events, 0.99),
        );
        assert_eq!(alive, 60, "{mode:?}: the healthy plugin's result wins every event");
        assert_eq!(healthy_chats, 100, "{mode:?}: the healthy plugin saw every event");
        assert!(recoveries >= 2, "{mode:?}: the faulty plugins did fault");
        assert!(
            percentile(&mut storm_commands, 0.99) < Duration::from_millis(250),
            "{mode:?}: the healthy plugin's own calls stay fast during the storm: p99 {:?}",
            percentile(&mut storm_commands, 0.99)
        );
    }
    .with_subscriber(logs.clone())
    .await;
    assert!(
        logs.matching("a host function panicked").is_empty(),
        "{:?}",
        logs.matching("a host function panicked")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panic_storm_in_two_plugins_leaves_a_healthy_plugin_correct_and_fast() {
    storm(Mode::Panic, "200ms").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cpu_spin_storm_in_two_plugins_leaves_a_healthy_plugin_correct_and_fast() {
    storm(Mode::Spin, "200ms").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_memory_storm_in_two_plugins_leaves_a_healthy_plugin_correct_and_fast() {
    storm(Mode::Grow, "200ms").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spinning_codec_filters_do_not_stall_a_healthy_plugin() {
    let lab = Arc::new(
        Lab::start(
            vec![LabPlugin::lab("").grant("codec-filter")],
            LabOptions {
                extra: vec![("scripted", HEALTHY, HEALTHY_SCRIPT.to_owned())],
                ..LabOptions::default()
            },
        )
        .await,
    );
    lab.enable(HEALTHY).await;
    let mut quiet = command_latencies(&lab, 20).await;

    let mut connections = Vec::new();
    for connection in 0..4u64 {
        let lab = Arc::clone(&lab);
        connections.push(tokio::spawn(async move {
            let (mut client, _server) = build_codec_chains(
                &lab.codecs,
                ProtocolVersion::new(767),
                connection + 100,
                "127.0.0.1:1".parse().unwrap(),
                None,
            );
            let mut packet = RawPacket::new(
                faults::CODEC_FAULT_PACKET_BASE + i32::try_from(Mode::Spin.code()).unwrap(),
                Bytes::from_static(b"x"),
            );
            let started = Instant::now();
            let _ = client.process(&mut packet);
            started.elapsed()
        }));
    }
    std::thread::sleep(Duration::from_millis(20));
    let started = Instant::now();
    lab.dispatch("greet").await;
    let stalled = started.elapsed();
    let mut spun = Vec::new();
    for connection in connections {
        spun.push(connection.await.unwrap());
    }
    let started = Instant::now();
    let _ = build_codec_chains(
        &lab.codecs,
        ProtocolVersion::new(767),
        faults::CODEC_FAULT_CONNECTION_BASE + Mode::Spin.code(),
        "127.0.0.1:1".parse().unwrap(),
        None,
    );
    let setup = started.elapsed();
    println!(
        "healthy command p50 quiet {:?}; during 4 spinning codec filters on 2 workers {stalled:?}; filter calls {spun:?}; one connection whose filter spins in create takes {setup:?} to set up",
        percentile(&mut quiet, 0.5)
    );
    assert!(
        stalled < Duration::from_millis(250),
        "a healthy plugin's command waited {stalled:?} behind codec filters of another plugin"
    );
    assert!(
        setup < Duration::from_millis(250),
        "a connection whose codec filter spins in create took {setup:?} to set up"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_named_event_chain_through_two_wasm_plugins_and_back_completes_without_deadlock() {
    let lab = Lab::start(
        vec![
            LabPlugin::lab("listen lab-pong"),
            LabPlugin::peer("relay lab-ping lab-pong"),
        ],
        LabOptions::default(),
    )
    .await;
    let took = lab.dispatch("labfire lab-ping").await;
    lab.wait_for("the event posted back to the busy plugin", || {
        lab.log(LAB).iter().any(|line| line == "named:lab-pong")
    })
    .await;
    assert!(took < Duration::from_secs(1), "A -> B -> A took {took:?}");
    assert!(
        lab.log(PEER).iter().any(|line| line == "fired lab-pong -"),
        "B's fire returned without waiting for the busy A: {:?}",
        lab.log(PEER)
    );
    assert!(
        lab.log(LAB).iter().any(|line| line == "fired lab-ping -"),
        "{:?}",
        lab.log(LAB)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn two_plugins_relaying_to_each_other_stop_by_themselves() {
    let logs = LogCapture::at(Level::WARN);
    let (first, second) = async {
        let lab = Lab::start(
            vec![
                LabPlugin::lab("relay lab-pong lab-ping"),
                LabPlugin::peer("relay lab-ping lab-pong"),
            ],
            LabOptions::default(),
        )
        .await;
        lab.dispatch("labfire lab-ping").await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let first = fault_lab::count(&lab.log(LAB), "named:lab-pong");
        tokio::time::sleep(Duration::from_secs(1)).await;
        let second = fault_lab::count(&lab.log(LAB), "named:lab-pong");
        (first, second)
    }
    .with_subscriber(logs.clone())
    .await;
    println!("relay hops seen by A: after 1s {first}, after 2s {second}");
    assert_eq!(
        first, second,
        "the relay between two busy plugins keeps going on its own ({first} then {second} hops)"
    );
    assert!(
        (1..=8).contains(&first),
        "the relay stops after a few hops: {first}"
    );
    assert_eq!(
        logs.matching("wasm plugin event dropped").len(),
        1,
        "the cut is logged once: {:?}",
        logs.lines()
    );
    let _ = PROMPTLY;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recovery_caused_by_a_waiting_plugin_does_not_wait_on_that_plugin() {
    let lab = Lab::start(
        vec![
            LabPlugin::peer("listen lab-y"),
            LabPlugin::lab("listen lab-x\nenable-fire lab-y"),
        ],
        LabOptions {
            bus: Some(infrarust_core::event_bus::EventBusConfig {
                handler_timeout: Duration::from_secs(3),
                ..infrarust_core::event_bus::EventBusConfig::default()
            }),
            ..LabOptions::default()
        },
    )
    .await;
    lab.set_faults(LAB, "listen lab-x\nenable-fire lab-y\nnamed:lab-x panic");
    let took = lab.dispatch("peerlabfire lab-x").await;
    lab.wait_for("A's recovery to finish its fire", || {
        lab.log(LAB)
            .iter()
            .any(|line| line.starts_with("fired lab-y"))
    })
    .await;
    println!(
        "B -> A (traps) -> A's recovery fires to B took {took:?}; A: {:?}; B: {:?}",
        lab.log(LAB),
        lab.log(PEER)
    );
    assert!(
        took < Duration::from_secs(1),
        "B waited {took:?}: A's recovery, done for B's call, waited on the busy B"
    );
}
