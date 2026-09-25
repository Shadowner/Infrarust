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
    let stats_target_dir = out_dir.join("stats-target");
    let stats_manifest = stats_dir.join("wasm").join("Cargo.toml");
    let fixtures = WasmBuild {
        label: "WASM fixtures",
        cwd: &fixtures_dir,
        target_dir: &fixture_target_dir,
        manifest: None,
        profile_overrides: &[],
    };
    let stats = WasmBuild {
        label: "stats plugin (wasm)",
        cwd: &repo_root,
        target_dir: &stats_target_dir,
        manifest: Some(&stats_manifest),
        profile_overrides: FAST_RELEASE_PROFILE,
    };

    let outcomes = std::thread::scope(|scope| {
        let stats_build = scope.spawn(|| stats.run(&cargo));
        let fixtures_outcome = fixtures.run(&cargo);
        let stats_outcome = stats_build
            .join()
            .expect("stats wasm build thread panicked");
        [(&fixtures, fixtures_outcome), (&stats, stats_outcome)]
    });
    for (build, outcome) in outcomes {
        match outcome {
            BuildOutcome::Built => {}
            BuildOutcome::TargetMissing => {
                println!(
                    "cargo:warning=wasm32-wasip2 target not installed; skipping {} (run `rustup target add wasm32-wasip2`). WASM tests will be skipped.",
                    build.label
                );
                return;
            }
            BuildOutcome::Failed(stderr) => {
                panic!(
                    "building {} for wasm32-wasip2 failed:\n{stderr}",
                    build.label
                )
            }
        }
    }

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

const FAST_RELEASE_PROFILE: &[(&str, &str)] = &[
    ("CARGO_PROFILE_RELEASE_OPT_LEVEL", "1"),
    ("CARGO_PROFILE_RELEASE_LTO", "false"),
    ("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "16"),
];

struct WasmBuild<'a> {
    label: &'a str,
    cwd: &'a Path,
    target_dir: &'a Path,
    manifest: Option<&'a Path>,
    profile_overrides: &'a [(&'a str, &'a str)],
}

enum BuildOutcome {
    Built,
    TargetMissing,
    Failed(String),
}

impl WasmBuild<'_> {
    fn run(&self, cargo: &str) -> BuildOutcome {
        let mut cmd = Command::new(cargo);
        cmd.current_dir(self.cwd).args([
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
            "--target-dir",
        ]);
        cmd.arg(self.target_dir);
        if let Some(manifest) = self.manifest {
            cmd.arg("--manifest-path").arg(manifest);
        }
        cmd.envs(self.profile_overrides.iter().copied());

        let output = match cmd.output() {
            Ok(output) => output,
            Err(e) => return BuildOutcome::Failed(format!("spawning cargo: {e}")),
        };
        if output.status.success() {
            return BuildOutcome::Built;
        }

        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if target_missing(&stderr) {
            BuildOutcome::TargetMissing
        } else {
            BuildOutcome::Failed(stderr)
        }
    }
}

fn target_missing(stderr: &str) -> bool {
    stderr.contains("may not be installed")
        || stderr.contains("can't find crate for `std`")
        || stderr.contains("the `wasm32-wasip2` target may not be installed")
}
