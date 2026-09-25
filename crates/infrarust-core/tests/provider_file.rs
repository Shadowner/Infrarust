#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::Path;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use infrarust_core::provider::file::FileProvider;
use infrarust_core::provider::{ConfigProvider, ProviderEvent};

const MINIMAL_CONFIG: &str = r#"
domains = ["test.example.com"]
addresses = ["127.0.0.1:25565"]
"#;

const FULL_CONFIG: &str = r#"
domains = ["survival.mc.com", "*.survival.mc.com"]
addresses = ["10.0.1.10:25565"]
proxy_mode = "passthrough"
"#;

fn create_test_config_dir(configs: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, content) in configs {
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
    }
    dir
}

#[tokio::test]
async fn test_load_initial_loads_all_toml() {
    let dir = create_test_config_dir(&[
        ("a.toml", MINIMAL_CONFIG),
        ("b.toml", MINIMAL_CONFIG),
        ("c.toml", FULL_CONFIG),
    ]);

    let provider = FileProvider::new(dir.path().to_path_buf());
    let configs = provider.load_initial().await.unwrap();
    assert_eq!(configs.len(), 3);
}

#[tokio::test]
async fn test_load_initial_ignores_non_toml() {
    let dir = create_test_config_dir(&[
        ("server.toml", MINIMAL_CONFIG),
        ("readme.txt", "not a config"),
        ("notes.md", "# notes"),
    ]);

    let provider = FileProvider::new(dir.path().to_path_buf());
    let configs = provider.load_initial().await.unwrap();
    assert_eq!(configs.len(), 1);
}

#[tokio::test]
async fn test_load_initial_invalid_toml_skipped() {
    let dir = create_test_config_dir(&[
        ("good.toml", MINIMAL_CONFIG),
        ("bad.toml", "this is not valid toml {{{}}}"),
    ]);

    let provider = FileProvider::new(dir.path().to_path_buf());
    let configs = provider.load_initial().await.unwrap();
    assert_eq!(configs.len(), 1);
}

#[tokio::test]
async fn test_load_initial_empty_dir() {
    let dir = create_test_config_dir(&[]);

    let provider = FileProvider::new(dir.path().to_path_buf());
    let configs = provider.load_initial().await.unwrap();
    assert!(configs.is_empty());
}

#[tokio::test]
async fn test_load_initial_missing_dir() {
    let provider = FileProvider::new("/nonexistent/path".into());
    let configs = provider.load_initial().await.unwrap();
    assert!(configs.is_empty());
}

#[tokio::test]
async fn test_provider_id_format() {
    let dir = create_test_config_dir(&[("survival.toml", MINIMAL_CONFIG)]);

    let provider = FileProvider::new(dir.path().to_path_buf());
    let configs = provider.load_initial().await.unwrap();
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].id.provider_type, "file");
    assert_eq!(configs[0].id.unique_id, "survival.toml");
    assert_eq!(configs[0].id.to_string(), "file@survival.toml");
}

fn summary(event: &ProviderEvent) -> String {
    match event {
        ProviderEvent::Added(pc) => format!("added {}", pc.id),
        ProviderEvent::Updated(pc) => format!("updated {}", pc.id),
        ProviderEvent::Removed(id) => format!("removed {id}"),
        ProviderEvent::Batch(events) => events.iter().map(summary).collect::<Vec<_>>().join(", "),
    }
}

async fn next_event(rx: &mut mpsc::Receiver<ProviderEvent>) -> ProviderEvent {
    let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timeout waiting for event")
        .expect("channel closed");
    match event {
        ProviderEvent::Batch(mut events) if events.len() == 1 => events.remove(0),
        event => event,
    }
}

struct Watching {
    rx: mpsc::Receiver<ProviderEvent>,
    shutdown: CancellationToken,
    handle: JoinHandle<()>,
}

impl Watching {
    async fn after_load(dir: &Path) -> Self {
        let provider = FileProvider::new(dir.to_path_buf());
        provider.load_initial().await.unwrap();
        std::fs::write(dir.join("ready.toml"), MINIMAL_CONFIG).unwrap();

        let (tx, mut rx) = mpsc::channel(32);
        let shutdown = CancellationToken::new();
        let token = shutdown.clone();
        let handle = tokio::spawn(async move {
            provider.watch(tx, token).await.unwrap();
        });
        assert_eq!(summary(&next_event(&mut rx).await), "added file@ready.toml");

        Self {
            rx,
            shutdown,
            handle,
        }
    }

    async fn next(&mut self) -> ProviderEvent {
        next_event(&mut self.rx).await
    }

    async fn only_the_probe_changes(&mut self, dir: &Path) {
        std::fs::write(dir.join("probe.toml"), MINIMAL_CONFIG).unwrap();
        assert_eq!(summary(&self.next().await), "added file@probe.toml");
        std::fs::remove_file(dir.join("probe.toml")).unwrap();
        assert_eq!(summary(&self.next().await), "removed file@probe.toml");
    }

    async fn stop(self) {
        self.shutdown.cancel();
        self.handle.await.unwrap();
    }
}

#[tokio::test]
async fn test_watch_detects_new_file() {
    let dir = create_test_config_dir(&[]);
    let mut watching = Watching::after_load(dir.path()).await;

    std::fs::write(dir.path().join("new.toml"), MINIMAL_CONFIG).unwrap();

    assert_eq!(summary(&watching.next().await), "added file@new.toml");
    watching.stop().await;
}

#[tokio::test]
async fn test_watch_detects_removed_file() {
    let dir = create_test_config_dir(&[("existing.toml", MINIMAL_CONFIG)]);
    let mut watching = Watching::after_load(dir.path()).await;

    std::fs::remove_file(dir.path().join("existing.toml")).unwrap();

    assert_eq!(
        summary(&watching.next().await),
        "removed file@existing.toml"
    );
    watching.stop().await;
}

#[tokio::test]
async fn test_watch_detects_modified_file() {
    let dir = create_test_config_dir(&[("server.toml", MINIMAL_CONFIG)]);
    let mut watching = Watching::after_load(dir.path()).await;

    std::fs::write(dir.path().join("server.toml"), FULL_CONFIG).unwrap();

    let event = watching.next().await;
    assert_eq!(summary(&event), "updated file@server.toml");
    if let ProviderEvent::Updated(pc) = event {
        assert!(pc.config.domains.contains(&"survival.mc.com".to_string()));
    }
    watching.stop().await;
}

#[tokio::test]
async fn test_watch_stops_on_shutdown() {
    let dir = create_test_config_dir(&[]);
    let provider = FileProvider::new(dir.path().to_path_buf());
    let (tx, _rx) = mpsc::channel(32);
    let shutdown = CancellationToken::new();

    let shutdown_clone = shutdown.clone();
    let handle = tokio::spawn(async move {
        provider.watch(tx, shutdown_clone).await.unwrap();
    });

    // Cancel immediately
    shutdown.cancel();

    // Should exit quickly
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("watch did not stop on shutdown")
        .unwrap();
}

#[tokio::test]
async fn test_watch_invalid_toml_keeps_old() {
    let dir = create_test_config_dir(&[("server.toml", MINIMAL_CONFIG)]);
    let mut watching = Watching::after_load(dir.path()).await;

    std::fs::write(dir.path().join("server.toml"), "invalid {{{ toml").unwrap();

    watching.only_the_probe_changes(dir.path()).await;
    watching.stop().await;
}

#[tokio::test]
async fn test_watch_is_silent_about_files_already_loaded() {
    let dir = create_test_config_dir(&[("a.toml", MINIMAL_CONFIG), ("b.toml", FULL_CONFIG)]);
    let mut watching = Watching::after_load(dir.path()).await;

    watching.only_the_probe_changes(dir.path()).await;
    watching.stop().await;
}

#[tokio::test]
async fn test_watch_ignores_a_rewrite_with_the_same_content() {
    let dir = create_test_config_dir(&[("server.toml", FULL_CONFIG)]);
    let mut watching = Watching::after_load(dir.path()).await;

    std::fs::write(dir.path().join("server.toml"), FULL_CONFIG).unwrap();

    watching.only_the_probe_changes(dir.path()).await;
    watching.stop().await;
}

#[tokio::test]
async fn test_watch_emits_changes_made_between_load_and_watch() {
    let dir = create_test_config_dir(&[
        ("kept.toml", MINIMAL_CONFIG),
        ("changed.toml", MINIMAL_CONFIG),
        ("gone.toml", MINIMAL_CONFIG),
    ]);
    let provider = FileProvider::new(dir.path().to_path_buf());
    assert_eq!(provider.load_initial().await.unwrap().len(), 3);

    std::fs::write(dir.path().join("added.toml"), MINIMAL_CONFIG).unwrap();
    std::fs::write(dir.path().join("changed.toml"), FULL_CONFIG).unwrap();
    std::fs::remove_file(dir.path().join("gone.toml")).unwrap();

    let (tx, mut rx) = mpsc::channel(32);
    let shutdown = CancellationToken::new();
    let shutdown_clone = shutdown.clone();
    let watch_handle = tokio::spawn(async move {
        provider.watch(tx, shutdown_clone).await.unwrap();
    });

    let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("a change made before the watch was lost")
        .expect("channel closed");
    let ProviderEvent::Batch(changes) = event else {
        panic!("expected one batch for one scan, got {}", summary(&event));
    };
    let mut seen = Vec::new();
    for change in &changes {
        if let ProviderEvent::Updated(pc) = change {
            assert!(pc.config.domains.contains(&"survival.mc.com".to_string()));
        }
        seen.push(summary(change));
    }
    seen.sort();
    assert_eq!(
        seen,
        [
            "added file@added.toml",
            "removed file@gone.toml",
            "updated file@changed.toml",
        ]
    );

    shutdown.cancel();
    watch_handle.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn test_watch_keeps_servers_when_the_directory_cannot_be_listed() {
    use std::os::unix::fs::PermissionsExt;

    let dir = create_test_config_dir(&[("a.toml", MINIMAL_CONFIG), ("b.toml", FULL_CONFIG)]);
    let mut watching = Watching::after_load(dir.path()).await;

    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o000)).unwrap();
    let during = tokio::time::timeout(Duration::from_secs(1), watching.rx.recv()).await;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        during.is_err(),
        "an unreadable directory changed the servers: {during:?}"
    );
    watching.only_the_probe_changes(dir.path()).await;
    watching.stop().await;
}
