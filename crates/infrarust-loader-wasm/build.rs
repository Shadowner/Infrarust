//! Builds the WASM-1 guest test fixtures for `wasm32-wasip2`.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(wasm_fixtures_available)");

    if std::env::var_os("CARGO_FEATURE_WASM").is_none() {
        return;
    }

    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is always set"),
    );
    let fixtures_dir = manifest_dir.join("tests").join("fixtures");
    let wit_dir = manifest_dir.join("..").join("infrarust-plugin-wit").join("wit");

    println!("cargo:rerun-if-changed={}", fixtures_dir.display());
    println!("cargo:rerun-if-changed={}", wit_dir.display());
    println!("cargo:rerun-if-env-changed=INFRARUST_WASM_FIXTURES_SKIP");

    if std::env::var_os("INFRARUST_WASM_FIXTURES_SKIP").is_some() {
        println!(
            "cargo:warning=INFRARUST_WASM_FIXTURES_SKIP is set; skipping WASM fixture build (fixture tests will be skipped)."
        );
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is always set"));
    // A dedicated target dir keeps the wasm build from contending on the host target lock.
    let fixture_target_dir = out_dir.join("fixture-target");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());

    let status = Command::new(&cargo)
        .current_dir(&fixtures_dir)
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
            "--target-dir",
        ])
        .arg(&fixture_target_dir)
        .status();

    match status {
        Ok(s) if s.success() => {
            let artifact_dir = fixture_target_dir.join("wasm32-wasip2").join("release");
            println!("cargo:rustc-cfg=wasm_fixtures_available");
            println!(
                "cargo:rustc-env=INFRARUST_WASM_FIXTURE_DIR={}",
                artifact_dir.display()
            );
        }
        _ => {
            println!(
                "cargo:warning=could not build wasm32-wasip2 fixtures (is the target installed? `rustup target add wasm32-wasip2`). WASM fixture tests will be skipped."
            );
        }
    }
}
