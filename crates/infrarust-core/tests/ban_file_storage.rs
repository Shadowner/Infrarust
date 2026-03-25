#![allow(clippy::unwrap_used, clippy::expect_used, clippy::unused_async)]
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use infrarust_core::ban::file_storage::FileBanStorage;
use infrarust_core::ban::storage::BanStorage;
use infrarust_core::ban::types::{BanEntry, BanTarget};
use uuid::Uuid;

async fn temp_storage() -> (FileBanStorage, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_bans.json");
    let storage = FileBanStorage::new(path);
    (storage, dir)
}

fn permanent_ban(target: BanTarget) -> BanEntry {
    BanEntry::new(target, Some("test".into()), None, "test".into())
}

fn temp_ban(target: BanTarget) -> BanEntry {
    BanEntry::new(
        target,
        Some("test".into()),
        Some(Duration::from_secs(3600)),
        "test".into(),
    )
}

fn expired_ban(target: BanTarget) -> BanEntry {
    BanEntry {
        target,
        reason: Some("test".into()),
        expires_at: Some(SystemTime::now() - Duration::from_secs(3600)),
        created_at: SystemTime::now() - Duration::from_secs(7200),
        source: "test".into(),
    }
}

#[tokio::test]
async fn test_add_and_check_ip_ban() {
    let (storage, _dir) = temp_storage().await;
    let ip: IpAddr = "192.168.1.100".parse().unwrap();

    storage
        .add_ban(permanent_ban(BanTarget::Ip(ip)))
        .await
        .unwrap();

    let result = storage.check_player(&ip, "someone", None).await.unwrap();
    assert!(result.is_some());
}

#[tokio::test]
async fn test_add_and_check_username_ban() {
    let (storage, _dir) = temp_storage().await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    storage
        .add_ban(permanent_ban(BanTarget::Username("Griefer".into())))
        .await
        .unwrap();

    let result = storage.check_player(&ip, "Griefer", None).await.unwrap();
    assert!(result.is_some());
}

#[tokio::test]
async fn test_add_and_check_uuid_ban() {
    let (storage, _dir) = temp_storage().await;
    let uuid = Uuid::new_v4();

    storage
        .add_ban(permanent_ban(BanTarget::Uuid(uuid)))
        .await
        .unwrap();

    let result = storage.is_banned(&BanTarget::Uuid(uuid)).await.unwrap();
    assert!(result.is_some());
}

#[tokio::test]
async fn test_username_case_insensitive() {
    let (storage, _dir) = temp_storage().await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    storage
        .add_ban(permanent_ban(BanTarget::Username("Steve".into())))
        .await
        .unwrap();

    let result = storage.check_player(&ip, "steve", None).await.unwrap();
    assert!(result.is_some());

    let result = storage.check_player(&ip, "STEVE", None).await.unwrap();
    assert!(result.is_some());
}

#[tokio::test]
async fn test_remove_ban() {
    let (storage, _dir) = temp_storage().await;
    let target = BanTarget::Username("Player".into());

    storage
        .add_ban(permanent_ban(target.clone()))
        .await
        .unwrap();
    assert!(storage.is_banned(&target).await.unwrap().is_some());

    let removed = storage.remove_ban(&target).await.unwrap();
    assert!(removed);
    assert!(storage.is_banned(&target).await.unwrap().is_none());
}

#[tokio::test]
async fn test_remove_nonexistent() {
    let (storage, _dir) = temp_storage().await;
    let removed = storage
        .remove_ban(&BanTarget::Username("nobody".into()))
        .await
        .unwrap();
    assert!(!removed);
}

#[tokio::test]
async fn test_replace_existing_ban() {
    let (storage, _dir) = temp_storage().await;
    let target = BanTarget::Username("Player".into());

    storage
        .add_ban(BanEntry::new(
            target.clone(),
            Some("reason1".into()),
            None,
            "test".into(),
        ))
        .await
        .unwrap();
    storage
        .add_ban(BanEntry::new(
            target.clone(),
            Some("reason2".into()),
            None,
            "test".into(),
        ))
        .await
        .unwrap();

    let entry = storage.is_banned(&target).await.unwrap().unwrap();
    assert_eq!(entry.reason.as_deref(), Some("reason2"));
}

#[tokio::test]
async fn test_expired_ban_not_returned() {
    let (storage, _dir) = temp_storage().await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    storage
        .add_ban(expired_ban(BanTarget::Ip(ip)))
        .await
        .unwrap();

    let result = storage.check_player(&ip, "someone", None).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_expired_ban_lazy_purge() {
    let (storage, _dir) = temp_storage().await;
    let target = BanTarget::Username("Expired".into());

    storage.add_ban(expired_ban(target.clone())).await.unwrap();

    // First access triggers lazy purge
    let result = storage.is_banned(&target).await.unwrap();
    assert!(result.is_none());

    // Should be gone from storage now
    let all = storage.get_all_active().await.unwrap();
    assert!(all.is_empty());
}

#[tokio::test]
async fn test_check_player_ip_priority() {
    let (storage, _dir) = temp_storage().await;
    let ip: IpAddr = "192.168.1.1".parse().unwrap();

    storage
        .add_ban(BanEntry::new(
            BanTarget::Ip(ip),
            Some("ip ban".into()),
            None,
            "test".into(),
        ))
        .await
        .unwrap();
    storage
        .add_ban(BanEntry::new(
            BanTarget::Username("Player".into()),
            Some("name ban".into()),
            None,
            "test".into(),
        ))
        .await
        .unwrap();

    let result = storage
        .check_player(&ip, "Player", None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.reason.as_deref(), Some("ip ban"));
}

#[tokio::test]
async fn test_check_player_no_uuid() {
    let (storage, _dir) = temp_storage().await;
    let uuid = Uuid::new_v4();
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    storage
        .add_ban(permanent_ban(BanTarget::Uuid(uuid)))
        .await
        .unwrap();

    // Without UUID, should not find the ban
    let result = storage.check_player(&ip, "someone", None).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_purge_expired() {
    let (storage, _dir) = temp_storage().await;

    // Add 2 expired + 1 active
    storage
        .add_ban(expired_ban(BanTarget::Ip("1.1.1.1".parse().unwrap())))
        .await
        .unwrap();
    storage
        .add_ban(expired_ban(BanTarget::Username("old".into())))
        .await
        .unwrap();
    storage
        .add_ban(permanent_ban(BanTarget::Username("active".into())))
        .await
        .unwrap();

    let purged = storage.purge_expired().await.unwrap();
    assert_eq!(purged, 2);

    let active = storage.get_all_active().await.unwrap();
    assert_eq!(active.len(), 1);
}

#[tokio::test]
async fn test_get_all_active() {
    let (storage, _dir) = temp_storage().await;

    storage
        .add_ban(permanent_ban(BanTarget::Ip("1.1.1.1".parse().unwrap())))
        .await
        .unwrap();
    storage
        .add_ban(temp_ban(BanTarget::Username("Player".into())))
        .await
        .unwrap();
    storage
        .add_ban(expired_ban(BanTarget::Username("Old".into())))
        .await
        .unwrap();

    let active = storage.get_all_active().await.unwrap();
    assert_eq!(active.len(), 2);
}

#[tokio::test]
async fn test_persistence_save_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("persist_bans.json");

    // Create and populate
    {
        let storage = FileBanStorage::new(path.clone());
        storage
            .add_ban(permanent_ban(BanTarget::Ip("5.5.5.5".parse().unwrap())))
            .await
            .unwrap();
        storage
            .add_ban(permanent_ban(BanTarget::Username("Persisted".into())))
            .await
            .unwrap();
        storage.save().await.unwrap();
    }

    // Load in new storage
    {
        let storage = FileBanStorage::new(path);
        storage.load().await.unwrap();
        let all = storage.get_all_active().await.unwrap();
        assert_eq!(all.len(), 2);

        let ip_ban = storage
            .is_banned(&BanTarget::Ip("5.5.5.5".parse().unwrap()))
            .await
            .unwrap();
        assert!(ip_ban.is_some());

        let name_ban = storage
            .is_banned(&BanTarget::Username("Persisted".into()))
            .await
            .unwrap();
        assert!(name_ban.is_some());
    }
}

#[tokio::test]
async fn test_empty_file_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nonexistent_bans.json");

    let storage = FileBanStorage::new(path);
    // Should not error on missing file
    storage.load().await.unwrap();
    let all = storage.get_all_active().await.unwrap();
    assert!(all.is_empty());
}

#[tokio::test]
async fn test_concurrent_access() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("concurrent_bans.json");
    let storage = Arc::new(FileBanStorage::new(path));

    let mut handles = Vec::new();

    for i in 0..100u32 {
        let s = Arc::clone(&storage);
        handles.push(tokio::spawn(async move {
            let ip: IpAddr = format!("10.0.{}.{}", i / 256, i % 256).parse().unwrap();
            let target = BanTarget::Ip(ip);
            s.add_ban(BanEntry::new(target.clone(), None, None, "test".into()))
                .await
                .unwrap();
            s.is_banned(&target).await.unwrap();
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    let all = storage.get_all_active().await.unwrap();
    assert_eq!(all.len(), 100);
}
