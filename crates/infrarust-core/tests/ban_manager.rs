#![allow(clippy::unwrap_used, clippy::expect_used, clippy::unused_async)]
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use infrarust_api::types::{GameProfile, PlayerId, ProtocolVersion, ServerId};
use infrarust_core::ban::file_storage::FileBanStorage;
use infrarust_core::ban::manager::BanManager;
use infrarust_core::ban::types::BanTarget;
use infrarust_core::player::PlayerSession;
use infrarust_core::registry::ConnectionRegistry;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

async fn temp_manager() -> (Arc<BanManager>, Arc<ConnectionRegistry>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_bans.json");
    let storage = Arc::new(FileBanStorage::new(path));
    let registry = Arc::new(ConnectionRegistry::new());
    let manager = Arc::new(BanManager::new(storage, Arc::clone(&registry)));
    (manager, registry, dir)
}

fn make_session(username: &str, ip: IpAddr) -> (Arc<PlayerSession>, CancellationToken) {
    let token = CancellationToken::new();
    let (tx, _rx) = mpsc::channel(32);
    let uuid = Uuid::new_v4();
    let session = Arc::new(PlayerSession::new(
        PlayerId::new(uuid.as_u128() as u64),
        GameProfile {
            uuid,
            username: username.to_string(),
            properties: vec![],
        },
        ProtocolVersion::new(767),
        std::net::SocketAddr::new(ip, 12345),
        Some(ServerId::new("test-server")),
        false,
        false,
        tx,
        token.clone(),
        infrarust_core::permissions::default_checker(),
    ));
    (session, token)
}

#[tokio::test]
async fn test_ban_and_kick_online_player() {
    let (manager, registry, _dir) = temp_manager().await;
    let ip: IpAddr = "192.168.1.50".parse().unwrap();
    let (session, token) = make_session("Victim", ip);
    registry.register(session);

    assert!(!token.is_cancelled());

    manager
        .ban(
            BanTarget::Username("Victim".into()),
            Some("bad behavior".into()),
            None,
            "admin".into(),
        )
        .await
        .unwrap();

    assert!(token.is_cancelled());
}

#[tokio::test]
async fn test_ban_ip_kicks_multiple() {
    let (manager, registry, _dir) = temp_manager().await;
    let ip: IpAddr = "10.0.0.5".parse().unwrap();

    let (s1, t1) = make_session("Player1", ip);
    let (s2, t2) = make_session("Player2", ip);
    let (s3, t3) = make_session("Player3", ip);
    registry.register(s1);
    registry.register(s2);
    registry.register(s3);

    manager
        .ban(
            BanTarget::Ip(ip),
            Some("shared IP".into()),
            None,
            "console".into(),
        )
        .await
        .unwrap();

    assert!(t1.is_cancelled());
    assert!(t2.is_cancelled());
    assert!(t3.is_cancelled());
}

#[tokio::test]
async fn test_ban_offline_player() {
    let (manager, _registry, _dir) = temp_manager().await;

    manager
        .ban(
            BanTarget::Username("OfflineGuy".into()),
            Some("reason".into()),
            None,
            "test".into(),
        )
        .await
        .unwrap();

    let ip: IpAddr = "10.0.0.1".parse().unwrap();
    let result = manager.check_player(&ip, "OfflineGuy", None).await.unwrap();
    assert!(result.is_some());
}

#[tokio::test]
async fn test_unban() {
    let (manager, _registry, _dir) = temp_manager().await;
    let target = BanTarget::Username("Temp".into());

    manager
        .ban(target.clone(), None, None, "test".into())
        .await
        .unwrap();

    let removed = manager.unban(&target).await.unwrap();
    assert!(removed);

    let ip: IpAddr = "10.0.0.1".parse().unwrap();
    let result = manager.check_player(&ip, "Temp", None).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_check_player_delegates() {
    let (manager, _registry, _dir) = temp_manager().await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    manager
        .ban(
            BanTarget::Ip(ip),
            Some("blocked".into()),
            None,
            "test".into(),
        )
        .await
        .unwrap();

    let result = manager.check_player(&ip, "anyone", None).await.unwrap();
    assert!(result.is_some());
}

#[tokio::test]
async fn test_purge_task_stops_on_shutdown() {
    let (manager, _registry, _dir) = temp_manager().await;
    let shutdown = CancellationToken::new();

    let handle = manager.start_purge_task(Duration::from_millis(50), shutdown.clone());

    // Let it run briefly
    tokio::time::sleep(Duration::from_millis(100)).await;

    shutdown.cancel();
    // Task should stop promptly
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("purge task should stop within timeout")
        .expect("purge task should not panic");
}
