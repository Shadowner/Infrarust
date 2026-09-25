#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::time::Duration;

use tokio::sync::mpsc;
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

#[tokio::test]
async fn test_watch_detects_new_file() {
    let dir = create_test_config_dir(&[]);
    let provider = FileProvider::new(dir.path().to_path_buf());
    let (tx, mut rx) = mpsc::channel(32);
    let shutdown = CancellationToken::new();

    let shutdown_clone = shutdown.clone();
    let watch_handle = tokio::spawn(async move {
        provider.watch(tx, shutdown_clone).await.unwrap();
    });

    // Wait for watcher to initialize
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Create a new config file
    std::fs::write(dir.path().join("new.toml"), MINIMAL_CONFIG).unwrap();

    // Wait for event
    let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timeout waiting for event")
        .expect("channel closed");

    assert!(matches!(event, ProviderEvent::Added(_)));
    if let ProviderEvent::Added(pc) = event {
        assert_eq!(pc.id.to_string(), "file@new.toml");
    }

    shutdown.cancel();
    watch_handle.await.unwrap();
}

#[tokio::test]
async fn test_watch_detects_removed_file() {
    let dir = create_test_config_dir(&[("existing.toml", MINIMAL_CONFIG)]);
    let provider = FileProvider::new(dir.path().to_path_buf());
    let (tx, mut rx) = mpsc::channel(32);
    let shutdown = CancellationToken::new();

    let shutdown_clone = shutdown.clone();
    let watch_handle = tokio::spawn(async move {
        provider.watch(tx, shutdown_clone).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Remove the file
    std::fs::remove_file(dir.path().join("existing.toml")).unwrap();

    let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timeout waiting for event")
        .expect("channel closed");

    assert!(matches!(event, ProviderEvent::Removed(_)));
    if let ProviderEvent::Removed(id) = event {
        assert_eq!(id.to_string(), "file@existing.toml");
    }

    shutdown.cancel();
    watch_handle.await.unwrap();
}

#[tokio::test]
async fn test_watch_detects_modified_file() {
    let dir = create_test_config_dir(&[("server.toml", MINIMAL_CONFIG)]);
    let provider = FileProvider::new(dir.path().to_path_buf());
    let (tx, mut rx) = mpsc::channel(32);
    let shutdown = CancellationToken::new();

    let shutdown_clone = shutdown.clone();
    let watch_handle = tokio::spawn(async move {
        provider.watch(tx, shutdown_clone).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Modify the file with different content
    std::fs::write(dir.path().join("server.toml"), FULL_CONFIG).unwrap();

    let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timeout waiting for event")
        .expect("channel closed");

    assert!(matches!(event, ProviderEvent::Updated(_)));
    if let ProviderEvent::Updated(pc) = event {
        assert_eq!(pc.id.to_string(), "file@server.toml");
        // Should have the updated domains
        assert!(pc.config.domains.contains(&"survival.mc.com".to_string()));
    }

    shutdown.cancel();
    watch_handle.await.unwrap();
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
    let provider = FileProvider::new(dir.path().to_path_buf());
    let (tx, mut rx) = mpsc::channel(32);
    let shutdown = CancellationToken::new();

    let shutdown_clone = shutdown.clone();
    let watch_handle = tokio::spawn(async move {
        provider.watch(tx, shutdown_clone).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Write invalid TOML — should NOT produce an Updated or Removed event
    std::fs::write(dir.path().join("server.toml"), "invalid {{{ toml").unwrap();

    // Should timeout — no event emitted for invalid TOML
    let result = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
    assert!(
        result.is_err(),
        "expected no event for invalid TOML modification"
    );

    shutdown.cancel();
    watch_handle.await.unwrap();
}

async fn first_event_after_probe(dir: &std::path::Path, rx: &mut mpsc::Receiver<ProviderEvent>) {
    std::fs::write(dir.join("probe.toml"), MINIMAL_CONFIG).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timeout waiting for the probe")
        .expect("channel closed");
    match event {
        ProviderEvent::Added(pc) => assert_eq!(pc.id.to_string(), "file@probe.toml"),
        other => panic!("expected only the probe to be added, got {other:?}"),
    }
}

#[tokio::test]
async fn test_watch_is_silent_about_files_present_at_start() {
    let dir = create_test_config_dir(&[("a.toml", MINIMAL_CONFIG), ("b.toml", FULL_CONFIG)]);
    let provider = FileProvider::new(dir.path().to_path_buf());
    let (tx, mut rx) = mpsc::channel(32);
    let shutdown = CancellationToken::new();

    let shutdown_clone = shutdown.clone();
    let watch_handle = tokio::spawn(async move {
        provider.watch(tx, shutdown_clone).await.unwrap();
    });

    tokio::time::sleep(Duration::from_secs(1)).await;
    first_event_after_probe(dir.path(), &mut rx).await;

    shutdown.cancel();
    watch_handle.await.unwrap();
}

#[tokio::test]
async fn test_watch_ignores_a_rewrite_with_the_same_content() {
    let dir = create_test_config_dir(&[("server.toml", FULL_CONFIG)]);
    let provider = FileProvider::new(dir.path().to_path_buf());
    let (tx, mut rx) = mpsc::channel(32);
    let shutdown = CancellationToken::new();

    let shutdown_clone = shutdown.clone();
    let watch_handle = tokio::spawn(async move {
        provider.watch(tx, shutdown_clone).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(300)).await;
    std::fs::write(dir.path().join("server.toml"), FULL_CONFIG).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    first_event_after_probe(dir.path(), &mut rx).await;

    shutdown.cancel();
    watch_handle.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn test_watch_keeps_servers_when_the_directory_cannot_be_listed() {
    use std::os::unix::fs::PermissionsExt;

    let dir = create_test_config_dir(&[("a.toml", MINIMAL_CONFIG), ("b.toml", FULL_CONFIG)]);
    let provider = FileProvider::new(dir.path().to_path_buf());
    let (tx, mut rx) = mpsc::channel(32);
    let shutdown = CancellationToken::new();

    let shutdown_clone = shutdown.clone();
    let watch_handle = tokio::spawn(async move {
        provider.watch(tx, shutdown_clone).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(300)).await;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o000)).unwrap();
    let during = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        during.is_err(),
        "an unreadable directory changed the servers: {during:?}"
    );
    first_event_after_probe(dir.path(), &mut rx).await;

    shutdown.cancel();
    watch_handle.await.unwrap();
}
