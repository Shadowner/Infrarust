use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use infrarust_api::events::named::NamedEvent;
use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::test_util::MockConfigService;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_plugin_common::capability::gates;

use crate::probe;
use crate::report::{Table, median, per_iter_ns, runtime, scaled};

const WORKERS: usize = 4;

async fn fire_named(bus: &EventBusImpl, name: &str, times: usize) -> (f64, String) {
    let event = NamedEvent::new(name, "text/plain", Bytes::from(times.to_string()));
    let started = Instant::now();
    let event = bus.fire(event).await;
    let elapsed = started.elapsed().as_nanos() as f64;
    let answer = event
        .response
        .as_ref()
        .map(|response| String::from_utf8_lossy(&response.payload).into_owned())
        .unwrap_or_default();
    (elapsed, answer)
}

async fn per_call_ns(
    bus: &EventBusImpl,
    name: &str,
    calls: usize,
    repeats: usize,
) -> (f64, String) {
    let mut base = Vec::with_capacity(repeats);
    let mut loaded = Vec::with_capacity(repeats);
    let mut answer = String::new();
    for _ in 0..2 {
        let _ = fire_named(bus, name, 1).await;
        let _ = fire_named(bus, name, calls).await;
    }
    for _ in 0..repeats {
        base.push(fire_named(bus, name, 1).await.0);
        let (elapsed, reply) = fire_named(bus, name, calls).await;
        loaded.push(elapsed);
        answer = reply;
    }
    (
        (median(&loaded) - median(&base)) / (calls.saturating_sub(1).max(1)) as f64,
        answer,
    )
}

pub(crate) fn run(runs: usize) {
    println!("## Host calls made from a WASM event handler\n");
    let mut table = Table::new(
        "one host call from the guest (median of repeated named events, minus the 1-call event)",
        "ns per call",
    );
    let mut native = Table::new("native equivalents", "ns per call");
    for _ in 0..runs {
        for players in [0usize, 100, 1_000] {
            let rt = runtime(WORKERS);
            let registry = probe::registry_with(players);
            let config =
                Arc::new(MockConfigService::new().with_value("perf.key", "a configured value"));
            let probe = rt.block_on(probe::load(registry.clone(), config.clone()));
            let bus = Arc::clone(&probe.env.event_bus);
            let heavy = players >= 100;
            let list_calls = if heavy {
                scaled(40).max(5)
            } else {
                scaled(2_000).max(100)
            };
            let cheap_calls = scaled(2_000).max(100);
            let repeats = 15;
            let results = rt.block_on(async move {
                tokio::spawn(async move {
                    let mut out = vec![(
                        "players.list",
                        per_call_ns(&bus, "perf.players.list", list_calls, repeats).await,
                    )];
                    if players == 0 {
                        for (label, name) in [
                            ("players.count", "perf.players.count"),
                            ("config.get", "perf.config.get"),
                            ("log trace!, level off", "perf.log.trace"),
                            ("log info!, level on", "perf.log.info"),
                            ("pure guest loop, 1k LCG steps, no host call", "perf.spin"),
                        ] {
                            out.push((label, per_call_ns(&bus, name, cheap_calls, repeats).await));
                        }
                    }
                    out
                })
                .await
                .unwrap()
            });
            for (op, (ns, answer)) in results {
                let row = if op == "players.list" {
                    format!("players.list with {players} players online (answer {answer})")
                } else {
                    op.to_owned()
                };
                table.record(&row, ns);
            }
            let iters = scaled(if heavy { 200 } else { 20_000 });
            let started = Instant::now();
            for _ in 0..iters {
                black_box(registry.get_all_players());
            }
            native.record(
                &format!("get_all_players with {players} players"),
                per_iter_ns(started, iters),
            );
            if players == 0 {
                native_gates(&mut native, config.as_ref());
            }
            drop(probe);
            rt.shutdown_background();
        }
    }
    table.print();
    native.print();
}

fn native_gates(native: &mut Table, config: &dyn ConfigService) {
    let iters = scaled(200_000);
    let started = Instant::now();
    for _ in 0..iters {
        black_box(config.get_value(black_box("perf.key")));
    }
    native.record("config get_value", per_iter_ns(started, iters));
    let started = Instant::now();
    for _ in 0..iters {
        black_box(gates::required(
            black_box("players"),
            black_box("players.list"),
        ));
    }
    native.record(
        "gates::required(\"players\", \"players.list\")",
        per_iter_ns(started, iters),
    );
    let started = Instant::now();
    for _ in 0..iters {
        black_box(gates::required(
            black_box("config-service"),
            black_box("get-value"),
        ));
    }
    native.record(
        "gates::required(\"config-service\", \"get-value\")",
        per_iter_ns(started, iters),
    );
    let caps = CapabilitySet::baseline();
    let started = Instant::now();
    for _ in 0..iters {
        black_box(caps.has(black_box(Capability::PlayerRead)));
    }
    native.record("CapabilitySet::has", per_iter_ns(started, iters));
    let started = Instant::now();
    for i in 0..iters {
        tracing::info!(plugin = "perf-probe", "perf probe info line {i}");
    }
    native.record(
        "tracing::info! to the same sink",
        per_iter_ns(started, iters),
    );
}
