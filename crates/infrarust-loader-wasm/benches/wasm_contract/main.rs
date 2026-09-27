#![allow(clippy::unwrap_used, clippy::expect_used)]

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
#[allow(unused_imports)]
#[path = "../../tests/support/mod.rs"]
mod support;

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
mod codec;
#[cfg(all(feature = "wasm", wasm_fixtures_available))]
mod events;
#[cfg(all(feature = "wasm", wasm_fixtures_available))]
mod hostcalls;
#[cfg(all(feature = "wasm", wasm_fixtures_available))]
mod probe;
#[cfg(all(feature = "wasm", wasm_fixtures_available))]
mod report;

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
fn main() {
    let scenarios: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with("--"))
        .collect();
    let wants = |name: &str| scenarios.is_empty() || scenarios.iter().any(|s| s == name);
    probe::install_sink();
    let runs = report::runs();
    report::banner(runs);
    if wants("events") {
        events::run(runs);
    }
    if wants("hostcalls") {
        hostcalls::run(runs);
    }
    if wants("codec") {
        codec::run(runs);
    }
}

#[cfg(not(all(feature = "wasm", wasm_fixtures_available)))]
fn main() {
    eprintln!("wasm_contract: requires `--features wasm` and built wasm fixtures (skipped)");
}
