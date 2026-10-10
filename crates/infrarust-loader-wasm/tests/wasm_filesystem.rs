#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/net.rs"]
mod net;
mod support;

use std::path::Path;

use net::{PROBE, enable_probe, try_enable_probe};
use support::log_capture::LogCapture;
use tracing::Level;
use tracing::instrument::WithSubscriber;

fn mounts_toml(mounts: &[(&Path, &str, bool)]) -> String {
    mounts
        .iter()
        .map(|(host, guest, read_only)| {
            format!(
                "[[plugins.{PROBE}.wasm.mounts]]\nhost = \"{}\"\nguest = \"{guest}\"\nread_only = {read_only}\n",
                host.display()
            )
        })
        .collect()
}

fn shared_tree() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("shared")).unwrap();
    std::fs::write(root.path().join("shared/greeting.txt"), "shared-data").unwrap();
    std::fs::write(root.path().join("secret.txt"), "host-secret").unwrap();
    root
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_mount_reads_the_host_file_and_refuses_writes() {
    let root = shared_tree();
    let shared = root.path().join("shared");
    let probe = enable_probe(
        &["filesystem-extended"],
        &mounts_toml(&[(&shared, "/shared", true)]),
    )
    .await;

    assert_eq!(
        probe.run("read /shared/greeting.txt").await,
        "ok shared-data"
    );
    assert!(
        probe
            .run("write /shared/new.txt nope")
            .await
            .starts_with("err ")
    );
    assert!(
        probe
            .run("write /shared/greeting.txt overwritten")
            .await
            .starts_with("err ")
    );
    assert!(!shared.join("new.txt").exists());
    assert_eq!(
        std::fs::read_to_string(shared.join("greeting.txt")).unwrap(),
        "shared-data"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn read_only_is_the_default() {
    let root = shared_tree();
    let shared = root.path().join("shared");
    let toml = format!(
        "[[plugins.{PROBE}.wasm.mounts]]\nhost = \"{}\"\nguest = \"/shared\"\n",
        shared.display()
    );
    let probe = enable_probe(&["filesystem-extended"], &toml).await;
    assert!(
        probe
            .run("write /shared/new.txt nope")
            .await
            .starts_with("err ")
    );
    assert!(!shared.join("new.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_write_mount_writes_files_the_host_sees() {
    let root = shared_tree();
    let out = root.path().join("shared");
    let probe = enable_probe(
        &["filesystem-extended"],
        &mounts_toml(&[(&out, "/out", false)]),
    )
    .await;

    assert_eq!(probe.run("write /out/made.txt from-guest").await, "ok ");
    assert_eq!(
        std::fs::read_to_string(out.join("made.txt")).unwrap(),
        "from-guest"
    );
    assert_eq!(probe.run("read /out/greeting.txt").await, "ok shared-data");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_the_capability_nothing_is_mounted() {
    let root = shared_tree();
    let shared = root.path().join("shared");
    let logs = LogCapture::at(Level::WARN);

    async {
        let probe = enable_probe(&[], &mounts_toml(&[(&shared, "/shared", false)])).await;
        assert_eq!(probe.run("exists /shared").await, "ok false");
        assert_eq!(probe.run("read /shared/greeting.txt").await, "err NotFound");
        assert_eq!(probe.run("write /shared/x.txt data").await, "err NotFound");
    }
    .with_subscriber(logs.clone())
    .await;

    assert!(!shared.join("x.txt").exists());
    let ignored = logs.matching("wasm.mounts is ignored");
    assert_eq!(ignored.len(), 1, "{:?}", logs.lines());
    assert!(
        ignored[0].contains("`filesystem-extended` capability"),
        "{ignored:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dot_dot_cannot_escape_a_mount_or_the_data_dir() {
    let root = shared_tree();
    let shared = root.path().join("shared");
    let probe = enable_probe(
        &["filesystem-extended"],
        &mounts_toml(&[(&shared, "/shared", false)]),
    )
    .await;
    std::fs::write(probe.plugins_dir.join("beside-data.txt"), "host-beside").unwrap();

    for path in [
        "/shared/../secret.txt",
        "/shared/../../secret.txt",
        "/shared/./../secret.txt",
        "../secret.txt",
        "/../beside-data.txt",
        "../beside-data.txt",
    ] {
        let outcome = probe.run(&format!("read {path}")).await;
        assert!(outcome.starts_with("err "), "{path}: {outcome}");
        assert!(!outcome.contains("host-"), "{path}: {outcome}");
    }
    assert!(
        probe
            .run("write /shared/../escaped.txt x")
            .await
            .starts_with("err ")
    );
    assert!(!root.path().join("escaped.txt").exists());
    assert_eq!(
        probe.run("read /shared/../shared/greeting.txt").await,
        "err PermissionDenied",
        "wasi refuses `..` even when the path would come back inside the mount"
    );

    std::os::unix::fs::symlink(root.path().join("secret.txt"), shared.join("absolute.txt"))
        .unwrap();
    std::os::unix::fs::symlink("../secret.txt", shared.join("relative.txt")).unwrap();
    for path in ["/shared/absolute.txt", "/shared/relative.txt"] {
        let outcome = probe.run(&format!("read {path}")).await;
        assert!(outcome.starts_with("err "), "{path}: {outcome}");
        assert!(!outcome.contains("host-"), "{path}: {outcome}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_data_dir_stays_at_the_root_next_to_the_mounts() {
    let root = shared_tree();
    let shared = root.path().join("shared");
    let probe = enable_probe(
        &["filesystem-extended"],
        &mounts_toml(&[(&shared, "/shared", true)]),
    )
    .await;

    assert_eq!(probe.run("write /note.txt data-dir").await, "ok ");
    assert_eq!(probe.run("write relative.txt also").await, "ok ");
    assert_eq!(
        std::fs::read_to_string(probe.data.join("note.txt")).unwrap(),
        "data-dir"
    );
    assert_eq!(
        std::fs::read_to_string(probe.data.join("relative.txt")).unwrap(),
        "also"
    );
    assert_eq!(probe.run("exists /shared/greeting.txt").await, "ok true");
    assert!(!shared.join("note.txt").exists());
    assert!(!probe.data.join("shared").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_host_directory_fails_the_load_naming_the_plugin_and_the_path() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("not-there");
    let error = try_enable_probe(
        &["filesystem-extended"],
        &mounts_toml(&[(&missing, "/shared", true)]),
    )
    .await
    .err()
    .expect("the load fails");
    assert!(error.contains(PROBE), "{error}");
    assert!(error.contains(&missing.display().to_string()), "{error}");
    assert!(error.contains("plugins.net-probe.wasm.mounts"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn mounts_survive_a_recovery() {
    let root = shared_tree();
    let shared = root.path().join("shared");
    let probe = enable_probe(
        &["filesystem-extended"],
        &mounts_toml(&[(&shared, "/shared", true)]),
    )
    .await;
    probe.trap().await;
    assert_eq!(
        probe.run("read /shared/greeting.txt").await,
        "ok shared-data"
    );
    assert!(
        probe
            .run("write /shared/new.txt nope")
            .await
            .starts_with("err ")
    );
    assert!(!shared.join("new.txt").exists());
}
