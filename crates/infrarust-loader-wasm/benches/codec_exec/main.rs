#![allow(clippy::unwrap_used, clippy::expect_used)]

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
#[allow(unused_imports)]
#[path = "../../tests/support/mod.rs"]
mod support;

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
#[allow(unused_imports)]
#[path = "../../tests/fault_lab/mod.rs"]
mod fault_lab;

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
mod study;

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
fn main() {
    study::main();
}

#[cfg(not(all(feature = "wasm", wasm_fixtures_available)))]
fn main() {
    eprintln!("codec_exec: requires `--features wasm` and built wasm fixtures (skipped)");
}
