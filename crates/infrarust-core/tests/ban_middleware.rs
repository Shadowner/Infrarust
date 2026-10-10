#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unused_async,
    clippy::panic
)]
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use infrarust_api::services::ban_service::BanRequest;
use infrarust_core::ban::FileBanStorage;
use infrarust_core::ban::manager::{BAN_CHECK_UNAVAILABLE, BanManager};
use infrarust_core::ban::storage::BanStorage;
use infrarust_core::ban::types::{BanEntry, BanSource, BanTarget};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::middleware::ban_check::BanCheckMiddleware;
use infrarust_core::middleware::ban_ip_check::BanIpCheckMiddleware;
use infrarust_core::pipeline::context::ConnectionContext;
use infrarust_core::pipeline::middleware::{Middleware, MiddlewareResult};
use infrarust_core::pipeline::types::{ConnectionIntent, HandshakeData, LoginData};
use infrarust_core::registry::ConnectionRegistry;
use infrarust_protocol::version::ProtocolVersion;

fn builtin(storage: Arc<dyn BanStorage>) -> Arc<BanManager> {
    Arc::new(BanManager::builtin(
        storage,
        Arc::new(ConnectionRegistry::new()),
        Arc::new(EventBusImpl::new()),
    ))
}

async fn setup_with_manager() -> (BanCheckMiddleware, Arc<BanManager>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let manager = builtin(Arc::new(FileBanStorage::new(
        dir.path().join("test_bans.json"),
    )));
    (BanCheckMiddleware::new(Arc::clone(&manager)), manager, dir)
}

async fn loopback(ip: IpAddr) -> ConnectionContext {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = listener.local_addr().unwrap();
    let _client = tokio::net::TcpStream::connect(local_addr).await.unwrap();
    let (stream, _peer) = listener.accept().await.unwrap();

    let peer: SocketAddr = SocketAddr::new(ip, 12345);
    let local: SocketAddr = "0.0.0.0:25565".parse().unwrap();
    ConnectionContext::new_for_test(stream, peer, ip, local)
}

async fn make_context_with_login(ip: IpAddr, username: &str) -> ConnectionContext {
    let mut ctx = loopback(ip).await;
    ctx.extensions.insert(LoginData {
        username: username.to_string(),
        player_uuid: None,
        profile_key: None,
    });
    ctx
}

async fn make_context_with_intent(ip: IpAddr, intent: ConnectionIntent) -> ConnectionContext {
    let mut ctx = loopback(ip).await;
    ctx.extensions.insert(HandshakeData {
        domain: "lobby.test".to_string(),
        raw_host: "lobby.test".to_string(),
        port: 25565,
        protocol_version: ProtocolVersion(767),
        intent,
        raw_packets: Vec::new(),
    });
    ctx
}

async fn ban(manager: &BanManager, target: BanTarget, reason: &str) {
    manager
        .issue(BanRequest::new(target).reason(reason))
        .await
        .unwrap();
}

fn kick_text(result: MiddlewareResult) -> String {
    match result {
        MiddlewareResult::Kick(reason) => reason.to_plain(),
        other => panic!("expected a kick, got {other:?}"),
    }
}

#[tokio::test]
async fn test_banned_ip_rejected() {
    let (middleware, manager, _dir) = setup_with_manager().await;
    let ip: IpAddr = "192.168.1.50".parse().unwrap();
    ban(&manager, BanTarget::Ip(ip), "bad IP").await;

    let mut ctx = make_context_with_login(ip, "Player").await;
    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(kick_text(result).contains("bad IP"));
}

#[tokio::test]
async fn test_banned_username_rejected() {
    let (middleware, manager, _dir) = setup_with_manager().await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();
    ban(&manager, BanTarget::Username("Cheater".into()), "cheating").await;

    let mut ctx = make_context_with_login(ip, "Cheater").await;
    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Kick(_)));
}

#[tokio::test]
async fn test_clean_player_continues() {
    let (middleware, _manager, _dir) = setup_with_manager().await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    let mut ctx = make_context_with_login(ip, "GoodPlayer").await;
    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));
}

#[tokio::test]
async fn test_expired_ban_continues() {
    let ip: IpAddr = "10.0.0.1".parse().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(FileBanStorage::new(dir.path().join("test_bans.json")));
    storage
        .add_ban(
            BanEntry::new(
                String::new(),
                BanTarget::Username("WasBanned".into()),
                BanSource::Console,
            )
            .reason("old")
            .created_at(SystemTime::now() - Duration::from_secs(3600))
            .expires_at(SystemTime::now() - Duration::from_secs(60)),
        )
        .await
        .unwrap();
    let mw = BanCheckMiddleware::new(builtin(storage));

    let mut ctx = make_context_with_login(ip, "WasBanned").await;
    let result = mw.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));
}

#[tokio::test]
async fn test_reject_message_contains_reason() {
    let (middleware, manager, _dir) = setup_with_manager().await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();
    ban(
        &manager,
        BanTarget::Username("Banned".into()),
        "Griefing the spawn",
    )
    .await;

    let mut ctx = make_context_with_login(ip, "Banned").await;
    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(kick_text(result).contains("Griefing the spawn"));
}

#[tokio::test]
async fn test_no_login_data_continues() {
    let (middleware, _manager, _dir) = setup_with_manager().await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    let mut ctx = loopback(ip).await;
    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));
}

#[tokio::test]
async fn a_login_is_refused_while_the_selected_provider_is_missing() {
    let manager = Arc::new(BanManager::plugin(
        "libertybans",
        Arc::new(ConnectionRegistry::new()),
        Arc::new(EventBusImpl::new()),
    ));
    let middleware = BanCheckMiddleware::new(manager);

    let mut ctx = make_context_with_login("10.0.0.1".parse().unwrap(), "Anyone").await;
    let result = middleware.process(&mut ctx).await.unwrap();
    assert_eq!(kick_text(result), BAN_CHECK_UNAVAILABLE);
}

#[tokio::test]
async fn disabled_bans_let_every_login_through() {
    let manager = Arc::new(BanManager::disabled(
        Arc::new(ConnectionRegistry::new()),
        Arc::new(EventBusImpl::new()),
    ));
    let middleware = BanCheckMiddleware::new(manager);

    let mut ctx = make_context_with_login("10.0.0.1".parse().unwrap(), "Anyone").await;
    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));
}

#[tokio::test]
async fn a_banned_address_gets_no_status_answer_but_may_try_to_log_in() {
    let (_login, manager, _dir) = setup_with_manager().await;
    let ip: IpAddr = "198.51.100.20".parse().unwrap();
    ban(
        &manager,
        BanTarget::IpRange("198.51.100.0/24".parse().unwrap()),
        "range",
    )
    .await;
    let gate = BanIpCheckMiddleware::new(Arc::clone(&manager));

    let mut status = make_context_with_intent(ip, ConnectionIntent::Status).await;
    assert!(matches!(
        gate.process(&mut status).await.unwrap(),
        MiddlewareResult::ShortCircuit
    ));

    let mut login = make_context_with_intent(ip, ConnectionIntent::Login).await;
    assert!(matches!(
        gate.process(&mut login).await.unwrap(),
        MiddlewareResult::Continue
    ));

    let mut elsewhere =
        make_context_with_intent("198.51.101.20".parse().unwrap(), ConnectionIntent::Status).await;
    assert!(matches!(
        gate.process(&mut elsewhere).await.unwrap(),
        MiddlewareResult::Continue
    ));
}

#[tokio::test]
async fn a_status_request_is_answered_while_the_selected_provider_is_missing() {
    let manager = Arc::new(BanManager::plugin(
        "libertybans",
        Arc::new(ConnectionRegistry::new()),
        Arc::new(EventBusImpl::new()),
    ));
    let gate = BanIpCheckMiddleware::new(manager);

    let mut status =
        make_context_with_intent("10.0.0.1".parse().unwrap(), ConnectionIntent::Status).await;
    assert!(matches!(
        gate.process(&mut status).await.unwrap(),
        MiddlewareResult::Continue
    ));
}
