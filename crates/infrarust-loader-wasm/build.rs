//! Builds the WASM guest test fixtures and the double-distributed `stats` plugin
//! for `wasm32-wasip2`.
//!

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(wasm_fixtures_available)");

    if std::env::var_os("CARGO_FEATURE_WASM").is_none() {
        return;
    }

    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is always set"),
    );
    let repo_root = manifest_dir.join("..").join("..");
    let fixtures_dir = manifest_dir.join("tests").join("fixtures");
    let wit_dir = manifest_dir
        .join("..")
        .join("infrarust-plugin-wit")
        .join("wit");
    let sdk_dir = manifest_dir.join("..").join("infrarust-plugin-sdk");
    let macros_dir = manifest_dir.join("..").join("infrarust-plugin-macros");
    let stats_dir = repo_root.join("plugins").join("infrarust-plugin-stats");

    for dir in [&fixtures_dir, &wit_dir, &sdk_dir, &macros_dir, &stats_dir] {
        println!("cargo:rerun-if-changed={}", dir.display());
    }
    println!("cargo:rerun-if-env-changed=INFRARUST_WASM_FIXTURES_SKIP");

    if std::env::var_os("INFRARUST_WASM_FIXTURES_SKIP").is_some() {
        println!(
            "cargo:warning=INFRARUST_WASM_FIXTURES_SKIP is set; skipping WASM fixture build (fixture tests will be skipped)."
        );
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is always set"));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());

    let fixture_target_dir = out_dir.join("fixture-target");
    let fixtures = build_wasm(
        &cargo,
        &fixtures_dir,
        &fixture_target_dir,
        "WASM fixtures",
        None,
    );
    let Some(()) = fixtures else {
        return; 
    };

    let stats_target_dir = out_dir.join("stats-target");
    let stats_manifest = stats_dir.join("wasm").join("Cargo.toml");
    let stats = build_wasm(
        &cargo,
        &repo_root,
        &stats_target_dir,
        "stats plugin (wasm)",
        Some(&stats_manifest),
    );
    let Some(()) = stats else {
        return;
    };

    let artifact_dir = fixture_target_dir.join("wasm32-wasip2").join("release");
    let staged = artifact_dir.join("fixture_stats.wasm");
    let built = stats_target_dir
        .join("wasm32-wasip2")
        .join("release")
        .join("infrarust_plugin_stats_wasm.wasm");
    std::fs::copy(&built, &staged)
        .unwrap_or_else(|e| panic!("staging {} -> {}: {e}", built.display(), staged.display()));

    println!("cargo:rustc-cfg=wasm_fixtures_available");
    println!(
        "cargo:rustc-env=INFRARUST_WASM_FIXTURE_DIR={}",
        artifact_dir.display()
    );
}

/// Returns `Some(())` on success, `None` if the only reason it failed is the
/// missing `wasm32-wasip2` target. Any other failure panics.
fn build_wasm(
    cargo: &str,
    cwd: &Path,
    target_dir: &Path,
    label: &str,
    manifest: Option<&Path>,
) -> Option<()> {
    let mut cmd = Command::new(cargo);
    cmd.current_dir(cwd).args([
        "build",
        "--release",
        "--target",
        "wasm32-wasip2",
        "--target-dir",
    ]);
    cmd.arg(target_dir);
    if let Some(manifest) = manifest {
        cmd.arg("--manifest-path").arg(manifest);
    }

    let output = cmd
        .output()
        .unwrap_or_else(|e| panic!("spawning cargo for {label}: {e}"));
    if output.status.success() {
        return Some(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if target_missing(&stderr) {
        println!(
            "cargo:warning=wasm32-wasip2 target not installed; skipping {label} (run `rustup target add wasm32-wasip2`). WASM tests will be skipped."
        );
        return None;
    }

    panic!("building {label} for wasm32-wasip2 failed:\n{stderr}");
}

fn target_missing(stderr: &str) -> bool {
    stderr.contains("may not be installed")
        || stderr.contains("can't find crate for `std`")
        || stderr.contains("the `wasm32-wasip2` target may not be installed")
}
