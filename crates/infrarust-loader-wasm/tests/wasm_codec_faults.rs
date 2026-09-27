#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod fault_lab;
mod support;

use std::time::{Duration, Instant};

use bytes::Bytes;
use infrarust_api::types::{ProtocolVersion, RawPacket};
use infrarust_core::filter::codec_chain::{CodecFilterChain, FilterResult, build_codec_chains};
use tracing::Level;
use tracing::instrument::WithSubscriber;

use fault_lab::faults::{self, Mode};
use fault_lab::{Lab, LabOptions, LabPlugin};
use support::log_capture::LogCapture;

const ATTACKER: &str = "203.0.113.7:40000";
const PLAYER: &str = "198.51.100.9:40000";
const TRAPPED: &str = "wasm codec filter trapped";
const QUARANTINED: &str = "wasm codec filter quarantined for a client address";

async fn lab(proxy_toml: &str) -> Lab {
    Lab::start(
        vec![LabPlugin::lab("").grant("codec-filter")],
        LabOptions {
            proxy_toml: proxy_toml.to_owned(),
            ..LabOptions::default()
        },
    )
    .await
}

fn chains(lab: &Lab, connection: u64, remote: &str) -> (CodecFilterChain, CodecFilterChain) {
    build_codec_chains(
        &lab.codecs,
        ProtocolVersion::new(767),
        connection,
        remote.parse().unwrap(),
        None,
    )
}

fn client(lab: &Lab, connection: u64, remote: &str) -> CodecFilterChain {
    chains(lab, connection, remote).0
}

fn send(chain: &mut CodecFilterChain, mode: Mode) -> FilterResult {
    let mut packet = RawPacket::new(
        faults::CODEC_FAULT_PACKET_BASE + i32::try_from(mode.code()).unwrap(),
        Bytes::from_static(b"x"),
    );
    chain.process(&mut packet)
}

fn mark(chain: &mut CodecFilterChain) -> (FilterResult, bool) {
    let mut packet = RawPacket::new(faults::CODEC_MARK_PACKET, Bytes::from_static(b"original"));
    let result = chain.process(&mut packet);
    let marked = &packet.data[..] == b"fault-lab";
    (result, marked)
}

fn filtered(chain: &mut CodecFilterChain) -> bool {
    let (result, marked) = mark(chain);
    matches!(result, FilterResult::Pass { .. }) && marked
}

fn trip(lab: &Lab, remote: &str, first_connection: u64) {
    for n in 0..5 {
        let mut chain = client(lab, first_connection + n, remote);
        send(&mut chain, Mode::Panic);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spinning_filter_is_cut_at_its_budget_and_the_log_says_why() {
    let logs = LogCapture::at(Level::WARN);
    let took = async {
        let lab = lab("").await;
        let mut spinning = client(&lab, 1, ATTACKER);
        let started = Instant::now();
        let result = send(&mut spinning, Mode::Spin);
        let took = started.elapsed();
        assert!(matches!(result, FilterResult::Pass { .. }));
        assert!(!filtered(&mut spinning), "the instance passes from then on");
        let mut panicking = client(&lab, 2, ATTACKER);
        send(&mut panicking, Mode::Panic);
        took
    }
    .with_subscriber(logs.clone())
    .await;
    assert!(
        took < Duration::from_millis(250),
        "a filter that spins was cut after {took:?}"
    );
    let trapped = logs.matching(TRAPPED);
    assert_eq!(trapped.len(), 2, "{trapped:?}");
    assert!(
        trapped[0].contains("ran past codec_cpu_budget (5ms)"),
        "{trapped:?}"
    );
    assert!(
        trapped[1].contains("hit an unreachable instruction"),
        "{trapped:?}"
    );
    assert!(
        trapped.iter().all(|line| line.contains("passing through")),
        "{trapped:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn faults_from_one_address_quarantine_the_filter_for_that_address_only() {
    let logs = LogCapture::at(Level::WARN);
    async {
        let lab = lab("").await;
        let mut attacker_live = client(&lab, 1, ATTACKER);
        let mut player_live = client(&lab, 2, PLAYER);
        assert!(filtered(&mut attacker_live));

        trip(&lab, ATTACKER, 10);

        assert!(
            !filtered(&mut attacker_live),
            "the quarantine cuts the live instances of the address"
        );
        assert!(
            filtered(&mut player_live),
            "another address keeps its filter"
        );
        assert!(
            !filtered(&mut client(&lab, 20, ATTACKER)),
            "a new connection from the address passes unfiltered"
        );
        assert!(filtered(&mut client(&lab, 21, PLAYER)));
    }
    .with_subscriber(logs.clone())
    .await;
    let quarantined = logs.matching(QUARANTINED);
    assert_eq!(quarantined.len(), 1, "{quarantined:?}");
    assert!(quarantined[0].contains("203.0.113.7"), "{quarantined:?}");
    assert!(
        quarantined[0].contains("hit an unreachable instruction"),
        "{quarantined:?}"
    );
    assert!(
        quarantined[0].contains("pass through unfiltered"),
        "{quarantined:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_quarantined_address_is_filtered_again_after_its_backoff() {
    let lab =
        lab("[wasm.codec_quarantine]\nbackoff_initial = \"300ms\"\nbackoff_max = \"1s\"\n").await;
    trip(&lab, ATTACKER, 1);
    assert!(!filtered(&mut client(&lab, 10, ATTACKER)));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        filtered(&mut client(&lab, 11, ATTACKER)),
        "the address gets its filter back once the backoff has passed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zero_faults_turns_the_codec_quarantine_off() {
    let lab = lab("[wasm.codec_quarantine]\nfaults = 0\n").await;
    trip(&lab, ATTACKER, 1);
    trip(&lab, ATTACKER, 10);
    assert!(filtered(&mut client(&lab, 20, ATTACKER)));
}

#[tokio::test(flavor = "multi_thread")]
async fn exhausted_instance_slots_fail_a_filter_open_and_log_at_a_bounded_rate() {
    let logs = LogCapture::at(Level::WARN);
    async {
        let lab = lab("[wasm]\ninstance_pool = 8\n").await;
        let mut held = Vec::new();
        let mut unfiltered = 0;
        for n in 0..40 {
            let (mut client_side, server_side) = chains(&lab, n, PLAYER);
            if !filtered(&mut client_side) {
                unfiltered += 1;
            }
            held.push((client_side, server_side));
        }
        assert!(unfiltered > 20, "{unfiltered} unfiltered");
        for (mut client_side, mut server_side) in held {
            client_side.close();
            server_side.close();
        }
    }
    .with_subscriber(logs.clone())
    .await;
    let failures = logs.matching("codec instance create failed");
    assert!(
        !failures.is_empty() && failures.len() <= 10,
        "{} create failure lines",
        failures.len()
    );
    assert!(
        failures.iter().all(|line| line.contains("passing through")),
        "{failures:?}"
    );
    assert!(
        logs.matching(QUARANTINED).is_empty(),
        "running out of slots is not a fault of the client"
    );
}
