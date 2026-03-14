use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use infrarust_core::ban::file_storage::FileBanStorage;
use infrarust_core::ban::manager::BanManager;
use infrarust_core::ban::types::BanTarget;
use infrarust_core::middleware::ban_check::BanCheckMiddleware;
use infrarust_core::pipeline::context::ConnectionContext;
use infrarust_core::pipeline::middleware::{Middleware, MiddlewareResult};
use infrarust_core::pipeline::types::LoginData;
use infrarust_core::registry::ConnectionRegistry;

async fn setup() -> (BanCheckMiddleware, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_bans.json");
    let storage = Arc::new(FileBanStorage::new(path));
    let registry = Arc::new(ConnectionRegistry::new());
    let manager = Arc::new(BanManager::new(storage, registry));
    (BanCheckMiddleware::new(manager), dir)
}

async fn setup_with_manager() -> (BanCheckMiddleware, Arc<BanManager>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_bans.json");
    let storage = Arc::new(FileBanStorage::new(path));
    let registry = Arc::new(ConnectionRegistry::new());
    let manager = Arc::new(BanManager::new(storage, registry));
    let middleware = BanCheckMiddleware::new(Arc::clone(&manager));
    (middleware, manager, dir)
}

/// Creates a test ConnectionContext using a loopback TCP connection.
async fn make_context_with_login(ip: IpAddr, username: &str) -> ConnectionContext {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = listener.local_addr().unwrap();
    let _client = tokio::net::TcpStream::connect(local_addr).await.unwrap();
    let (stream, _peer) = listener.accept().await.unwrap();

    let peer: SocketAddr = format!("{ip}:12345").parse().unwrap();
    let local: SocketAddr = "0.0.0.0:25565".parse().unwrap();
    let mut ctx = ConnectionContext::new_for_test(stream, peer, ip, local);
    ctx.extensions.insert(LoginData {
        username: username.to_string(),
        player_uuid: None,
    });
    ctx
}

async fn make_context_without_login(ip: IpAddr) -> ConnectionContext {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = listener.local_addr().unwrap();
    let _client = tokio::net::TcpStream::connect(local_addr).await.unwrap();
    let (stream, _peer) = listener.accept().await.unwrap();

    let peer: SocketAddr = format!("{ip}:12345").parse().unwrap();
    let local: SocketAddr = "0.0.0.0:25565".parse().unwrap();
    ConnectionContext::new_for_test(stream, peer, ip, local)
}

#[tokio::test]
async fn test_banned_ip_rejected() {
    let (middleware, manager, _dir) = setup_with_manager().await;
    let ip: IpAddr = "192.168.1.50".parse().unwrap();

    manager
        .ban(
            BanTarget::Ip(ip),
            Some("bad IP".into()),
            None,
            "test".into(),
        )
        .await
        .unwrap();

    let mut ctx = make_context_with_login(ip, "Player").await;
    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Reject(_)));
}

#[tokio::test]
async fn test_banned_username_rejected() {
    let (middleware, manager, _dir) = setup_with_manager().await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    manager
        .ban(
            BanTarget::Username("Cheater".into()),
            Some("cheating".into()),
            None,
            "test".into(),
        )
        .await
        .unwrap();

    let mut ctx = make_context_with_login(ip, "Cheater").await;
    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Reject(_)));
}

#[tokio::test]
async fn test_clean_player_continues() {
    let (middleware, _dir) = setup().await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    let mut ctx = make_context_with_login(ip, "GoodPlayer").await;
    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));
}

#[tokio::test]
async fn test_expired_ban_continues() {
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    use infrarust_core::ban::storage::BanStorage;
    use infrarust_core::ban::types::BanEntry;
    use std::time::SystemTime;

    let entry = BanEntry {
        target: BanTarget::Username("WasBanned".into()),
        reason: Some("old".into()),
        expires_at: Some(SystemTime::now() - Duration::from_secs(60)),
        created_at: SystemTime::now() - Duration::from_secs(3600),
        source: "test".into(),
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_bans.json");
    let storage = Arc::new(FileBanStorage::new(path));
    storage.add_ban(entry).await.unwrap();
    let registry = Arc::new(ConnectionRegistry::new());
    let mgr = Arc::new(BanManager::new(storage, registry));
    let mw = BanCheckMiddleware::new(mgr);

    let mut ctx = make_context_with_login(ip, "WasBanned").await;
    let result = mw.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));
}

#[tokio::test]
async fn test_reject_message_contains_reason() {
    let (middleware, manager, _dir) = setup_with_manager().await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    manager
        .ban(
            BanTarget::Username("Banned".into()),
            Some("Griefing the spawn".into()),
            None,
            "test".into(),
        )
        .await
        .unwrap();

    let mut ctx = make_context_with_login(ip, "Banned").await;
    let result = middleware.process(&mut ctx).await.unwrap();
    match result {
        MiddlewareResult::Reject(msg) => {
            assert!(msg.contains("Griefing the spawn"));
        }
        _ => panic!("expected Reject"),
    }
}

#[tokio::test]
async fn test_no_login_data_continues() {
    let (middleware, _dir) = setup().await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    let mut ctx = make_context_without_login(ip).await;
    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));
}
