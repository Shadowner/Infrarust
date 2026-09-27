#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/net.rs"]
mod net;
mod support;

use std::path::Path;

use infrarust_api::loader::PluginContextFactory;
use net::{PROBE, enable_probe};
use support::{EnvOptions, make_env_with};

fn mounts_toml(host: &Path, guest: &str, read_only: bool) -> String {
    format!(
        "[[plugins.{PROBE}.wasm.mounts]]\nhost = \"{}\"\nguest = \"{guest}\"\nread_only = {read_only}\n",
        host.display()
    )
}

fn shared_tree() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("shared")).unwrap();
    std::fs::write(root.path().join("shared/greeting.txt"), "shared-data").unwrap();
    std::fs::write(root.path().join("secret.txt"), "host-secret").unwrap();
    root
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_mount_refuses_rename_truncate_and_delete() {
    let root = shared_tree();
    let shared = root.path().join("shared");
    let probe = enable_probe(
        &["filesystem-extended"],
        &mounts_toml(&shared, "/shared", true),
    )
    .await;

    assert!(
        probe
            .run("truncate /shared/greeting.txt")
            .await
            .starts_with("err "),
        "truncate must be refused on a read-only mount"
    );
    assert!(
        probe
            .run("rename /shared/greeting.txt /shared/moved.txt")
            .await
            .starts_with("err "),
        "rename must be refused on a read-only mount"
    );
    assert!(
        probe
            .run("remove /shared/greeting.txt")
            .await
            .starts_with("err "),
        "delete must be refused on a read-only mount"
    );
    assert_eq!(
        std::fs::read_to_string(shared.join("greeting.txt")).unwrap(),
        "shared-data",
        "the host file is unchanged"
    );
    assert!(!shared.join("moved.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_guest_created_symlink_cannot_escape_the_data_dir() {
    let root = shared_tree();
    let shared = root.path().join("shared");
    let probe = enable_probe(
        &["filesystem-extended"],
        &mounts_toml(&shared, "/shared", false),
    )
    .await;
    std::fs::write(probe.plugins_dir.join("beside-data.txt"), "host-beside").unwrap();

    let created = probe.run("symlink ../secret.txt climb.txt").await;
    assert!(
        created == "ok " || created.starts_with("err "),
        "symlink outcome: {created}"
    );
    let read = probe.run("read climb.txt").await;
    assert!(
        read.starts_with("err ") && !read.contains("host-"),
        "a guest symlink to ../secret.txt must not read outside content: {read}"
    );

    let absolute = probe.run("symlink /secret.txt abs.txt").await;
    assert!(
        absolute == "ok " || absolute.starts_with("err "),
        "symlink outcome: {absolute}"
    );
    let read_abs = probe.run("read abs.txt").await;
    assert!(
        read_abs.starts_with("err ") && !read_abs.contains("host-"),
        "a guest symlink to an absolute host path must not read outside content: {read_abs}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_mount_symlink_to_a_writable_host_dir_still_refuses_writes() {
    let root = shared_tree();
    let shared = root.path().join("shared");
    let writable = root.path().join("writable");
    std::fs::create_dir(&writable).unwrap();
    std::os::unix::fs::symlink(&writable, shared.join("escape")).unwrap();

    let probe = enable_probe(
        &["filesystem-extended"],
        &mounts_toml(&shared, "/shared", true),
    )
    .await;

    assert!(
        probe
            .run("write /shared/escape/pwned.txt data")
            .await
            .starts_with("err "),
        "a symlink out of a read-only mount must not become a write path"
    );
    assert!(!writable.join("pwned.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-31: host hard link in the data dir is readable"]
async fn a_host_hard_link_in_the_data_dir_exposes_an_outside_file() {
    let root = shared_tree();
    let probe = enable_probe(&[], "").await;
    let link = probe.data.join("linked.txt");
    std::fs::hard_link(root.path().join("secret.txt"), &link).unwrap();

    let read = probe.run("read /linked.txt").await;
    assert!(
        !read.contains("host-secret"),
        "a hard link placed in the data dir let the guest read an outside file: {read}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-30: no disk quota on the data dir"]
async fn a_guest_can_fill_its_data_dir_without_a_quota() {
    let probe = enable_probe(&[], "").await;
    let written = probe.run("fill big.bin 32").await;
    assert!(
        written.starts_with("err "),
        "a plugin filled 32 MiB into its data dir unbounded (no disk quota): {written}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-03: plugin id not validated by the host"]
async fn a_hostile_plugin_id_cannot_place_the_data_dir_outside_plugins_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let env = make_env_with(plugins_dir.clone(), EnvOptions::default());

    let ctx = env.factory.create_context("../sec-escape");
    let data_dir = ctx.data_dir();
    let canonical_plugins = plugins_dir.canonicalize().unwrap();
    let escaped = data_dir
        .canonicalize()
        .map(|dir| !dir.starts_with(&canonical_plugins))
        .unwrap_or(false);
    if let Ok(dir) = data_dir.canonicalize() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(
        !escaped,
        "a hostile plugin id placed its data dir outside plugins_dir: {}",
        data_dir.display()
    );
}
