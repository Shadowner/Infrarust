#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::time::Duration;

use super::legacy::LEGACY_STORE;
use super::store::file_stem;
use super::*;

const SURVIVAL: &str = r#"
    domains = ["mc.example.com"]
    addresses = ["10.0.0.1:25565", { address = "10.0.0.2:25565", weight = 3 }]
    balance = "least_conn"
    slow_start = "45s"
    slow_start_aggression = 2.5

    [active_health]
    kind = "status_ping"
    probe_healthy = true

    [motd.online]
    text = "Welcome"
    max_players = 42
"#;

fn open() -> (tempfile::TempDir, ServerDir) {
    let root = tempfile::tempdir().unwrap();
    let dir = ServerDir::open(root.path()).unwrap();
    (root, dir)
}

fn id(id: &str) -> DocumentId {
    DocumentId::new(id).unwrap()
}

/// The API's own write path: serialize what was submitted, then file it.
async fn store(dir: &ServerDir, id: &str, source: &str) -> String {
    let config = parse_document(id, source).unwrap();
    let text = to_document_text(&config).unwrap();
    dir.create(&DocumentId::new(id).unwrap(), text)
        .await
        .unwrap()
        .id()
        .to_string()
}

#[tokio::test]
async fn write_returns_the_id_and_produces_no_diff() {
    let (_root, dir) = open();

    assert_eq!(store(&dir, "survival", SURVIVAL).await, "survival");
    assert!(dir.diff().await.is_empty());
    assert!(dir.owns("survival"));
}

/// A document naming itself through `name` keeps `id` unset, which is the
/// shape `settle_id` leaves it in. What the store remembers must still be
/// what a rescan reads back, or every write echoes as an update.
#[tokio::test]
async fn write_of_a_name_only_document_produces_no_diff() {
    let (_root, dir) = open();
    let text = "name = \"survival\"\ndomains = [\"mc.example.com\"]\n\
                addresses = [\"10.0.0.1:25565\"]\n"
        .to_string();

    let committed = dir.create(&id("survival"), text).await.unwrap();
    assert_eq!(committed.id().as_str(), "survival");
    drop(committed);

    let changes: Vec<String> = dir.diff().await.iter().map(describe).collect();
    assert!(changes.is_empty(), "{changes:?}");
}

#[tokio::test]
async fn create_refuses_a_name_another_document_already_uses() {
    let (root, dir) = open();
    let path = root.path().join("servers/foo.toml");
    std::fs::write(
        &path,
        "name = \"bar\"\ndomains = [\"bar.example.com\"]\naddresses = [\"10.0.0.9:25565\"]\n",
    )
    .unwrap();
    dir.diff().await;

    let text = "id = \"foo\"\ndomains = [\"foo.example.com\"]\n\
                addresses = [\"10.0.0.1:25565\"]\n"
        .to_string();
    let error = dir.create(&id("foo"), text).await.unwrap_err();

    assert!(matches!(error, StoreError::Occupied(_)), "{error:?}");
    assert!(std::fs::read_to_string(&path).unwrap().contains("bar"));
}

#[tokio::test]
async fn a_document_filed_under_another_name_is_not_owned() {
    let (root, dir) = open();
    std::fs::write(
        root.path().join("servers/foo.toml"),
        "name = \"bar\"\ndomains = [\"bar.example.com\"]\naddresses = [\"10.0.0.9:25565\"]\n",
    )
    .unwrap();
    dir.diff().await;

    assert_eq!(dir.ownership(&id("foo")), Ownership::Shadowed);
    assert_eq!(dir.ownership(&id("bar")), Ownership::Shadowed);
    assert!(!dir.owns("foo"));
    assert!(!dir.owns("bar"));
}

#[tokio::test]
async fn a_document_cannot_be_filed_under_a_name_it_does_not_claim() {
    let (_root, dir) = open();
    let text = "name = \"bar\"\ndomains = [\"bar.example.com\"]\n\
                addresses = [\"10.0.0.9:25565\"]\n"
        .to_string();

    let error = dir.create(&id("foo"), text).await.unwrap_err();

    assert!(matches!(error, StoreError::IdMismatch { .. }), "{error:?}");
    assert!(!_root.path().join("servers/foo.toml").exists());
}

#[tokio::test]
async fn stored_document_keeps_every_field() {
    let (root, dir) = open();
    store(&dir, "survival", SURVIVAL).await;

    let reopened = ServerDir::open(root.path()).unwrap();
    let document = reopened.document_text("survival").unwrap();
    let config = parse_document("survival", &document).unwrap();

    assert_eq!(config.balance, infrarust_config::BalanceStrategy::LeastConn);
    assert_eq!(config.slow_start, Some(Duration::from_secs(45)));
    assert!((config.slow_start_aggression - 2.5).abs() < f64::EPSILON);
    assert_eq!(config.addresses[1].weight, 3);
    assert_eq!(
        config.active_health.as_ref().unwrap().kind,
        infrarust_config::ProbeKind::StatusPing
    );
    assert!(config.active_health.as_ref().unwrap().probe_healthy);
    assert_eq!(config.motd.online.as_ref().unwrap().max_players, Some(42));
    assert!(reopened.diff().await.is_empty());
}

#[tokio::test]
async fn diff_reports_added_updated_and_removed() {
    let (root, dir) = open();
    let path = root.path().join("servers/lobby.toml");

    std::fs::write(
        &path,
        "domains = [\"a.example.com\"]\naddresses = [\"10.0.0.1:1\"]\n",
    )
    .unwrap();
    let changes = dir.diff().await;
    assert_eq!(changes.len(), 1);
    assert!(matches!(&changes[0], DirChange::Added(doc) if doc.id.as_str() == "lobby"));

    std::fs::write(
        &path,
        "domains = [\"a.example.com\"]\naddresses = [\"10.0.0.1:2\"]\n",
    )
    .unwrap();
    let changes = dir.diff().await;
    assert_eq!(changes.len(), 1);
    assert!(matches!(&changes[0], DirChange::Updated(_)));

    assert!(dir.diff().await.is_empty());

    std::fs::remove_file(&path).unwrap();
    let changes = dir.diff().await;
    assert_eq!(changes.len(), 1);
    assert!(matches!(&changes[0], DirChange::Removed(id) if id.as_str() == "lobby"));
}

#[test]
fn an_unreadable_directory_is_not_an_empty_one() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("servers"), b"not a directory").unwrap();

    assert!(matches!(
        ServerDir::open(root.path()),
        Err(StoreError::Io { .. })
    ));
}

/// A directory that cannot be listed says nothing about its servers, so it
/// must not unregister them.
#[tokio::test]
async fn an_unreadable_directory_removes_nothing() {
    let (root, dir) = open();
    store(&dir, "survival", SURVIVAL).await;

    std::fs::remove_dir_all(root.path().join("servers")).unwrap();

    let changes: Vec<String> = dir.diff().await.iter().map(describe).collect();
    assert!(changes.is_empty(), "{changes:?}");
    assert!(dir.owns("survival"));
}

#[tokio::test]
async fn invalid_file_keeps_the_previous_version() {
    let (root, dir) = open();
    let path = root.path().join("servers/lobby.toml");

    std::fs::write(
        &path,
        "domains = [\"a.example.com\"]\naddresses = [\"10.0.0.1:1\"]\n",
    )
    .unwrap();
    assert_eq!(dir.diff().await.len(), 1);

    std::fs::write(&path, "addresses = [").unwrap();
    assert!(dir.diff().await.is_empty());
    assert!(dir.owns("lobby"));
}

#[tokio::test]
async fn remove_deletes_the_file() {
    let (root, dir) = open();
    store(&dir, "survival", SURVIVAL).await;

    let removed = dir
        .lock_writes()
        .await
        .remove(&id("survival"))
        .await
        .unwrap();
    assert_eq!(
        removed.map(|removed| removed.id().to_string()).as_deref(),
        Some("survival")
    );
    assert!(!root.path().join("servers/survival.toml").exists());
    assert!(
        dir.lock_writes()
            .await
            .remove(&id("survival"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(dir.diff().await.is_empty());
}

/// A file that appears before the proxy connects its sender gets no second
/// watcher event, so the initial hand-over has to rescan.
#[tokio::test]
async fn initial_documents_see_files_written_after_open() {
    let (root, dir) = open();
    std::fs::write(
        root.path().join("servers/lobby.toml"),
        "domains = [\"a.example.com\"]\naddresses = [\"10.0.0.1:1\"]\n",
    )
    .unwrap();

    let documents = dir.initial_documents().await;
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].id.as_str(), "lobby");
}

#[tokio::test]
async fn reload_without_a_sender_reports_nothing_and_keeps_the_change_pending() {
    let (root, dir) = open();
    let slot: ProviderSenderSlot = tokio::sync::Mutex::new(None);
    std::fs::write(
        root.path().join("servers/lobby.toml"),
        "domains = [\"a.example.com\"]\naddresses = [\"10.0.0.1:1\"]\n",
    )
    .unwrap();

    assert!(reload(&dir, &slot).await.is_none());

    let sender = test_support::RecordingSender::default();
    *slot.lock().await = Some(Box::new(sender.clone()));
    let summary = reload(&dir, &slot).await.unwrap();

    assert_eq!(summary.added, 1);
    assert_eq!(sender.events(), vec!["added:lobby".to_string()]);
}

#[test]
fn document_ids_round_trip_through_a_file_name() {
    let (root, dir) = open();
    let servers = root.path().join("servers");

    for candidate in ["lobby", "mc.lobby", "a-b_c", "9"] {
        let document = id(candidate);
        let path = dir.path_of(&document);
        assert_eq!(path.parent(), Some(servers.as_path()), "{candidate}");
        assert_eq!(file_stem(&path), Some(candidate), "{candidate}");
    }
}

#[test]
fn document_ids_reject_anything_a_file_name_cannot_carry() {
    for candidate in [
        "", "..", ".", "../x", "a/b", "C:\\evil", ".hidden", "a..b", "UPPER", "a b", "e\u{301}",
    ] {
        assert_eq!(DocumentId::new(candidate), None, "{candidate}");
    }
    assert_eq!(
        DocumentId::new(&"a".repeat(infrarust_config::MAX_IDENTIFIER_LEN + 1)),
        None
    );
}

#[test]
fn legacy_store_is_migrated() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join(LEGACY_STORE),
        r#"[{"id":"lobby","domains":["lobby.example.com"],
             "addresses":["127.0.0.1:25565"],"proxy_mode":"zerocopy",
             "limbo_handlers":["queue"]}]"#,
    )
    .unwrap();

    let dir = ServerDir::open(root.path()).unwrap();

    assert!(root.path().join("servers.json.migrated").exists());
    assert!(!root.path().join(LEGACY_STORE).exists());

    let config = parse_document(
        "lobby",
        &dir.document_text("lobby").expect("migrated document"),
    )
    .unwrap();
    assert_eq!(config.proxy_mode, infrarust_config::ProxyMode::ZeroCopy);
    assert_eq!(config.domains, vec!["lobby.example.com".to_string()]);
    assert_eq!(config.limbo_handlers, vec!["queue".to_string()]);
}

/// A half-migrated store that is renamed away leaves the operator nothing
/// to retry from.
#[test]
fn a_partly_migrated_legacy_store_is_kept() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join(LEGACY_STORE),
        r#"[{"id":"lobby","domains":["lobby.example.com"],
             "addresses":["127.0.0.1:25565"],"proxy_mode":"passthrough"},
            {"id":"My Server","domains":["mine.example.com"],
             "addresses":["127.0.0.1:25566"],"proxy_mode":"passthrough"}]"#,
    )
    .unwrap();

    let dir = ServerDir::open(root.path()).unwrap();

    assert!(dir.owns("lobby"));
    assert!(root.path().join(LEGACY_STORE).exists());
    assert!(!root.path().join("servers.json.migrated").exists());
}

fn describe(change: &DirChange) -> String {
    match change {
        DirChange::Added(doc) => format!("added:{}", doc.id.as_str()),
        DirChange::Updated(doc) => format!("updated:{}", doc.id.as_str()),
        DirChange::Removed(id) => format!("removed:{}", id.as_str()),
    }
}
