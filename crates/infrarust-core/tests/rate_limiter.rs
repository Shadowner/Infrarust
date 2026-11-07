#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_config::RateLimitConfig;
use infrarust_core::middleware::rate_limiter::RateLimiterMiddleware;
use infrarust_core::pipeline::context::ConnectionContext;
use infrarust_core::pipeline::middleware::{Middleware, MiddlewareResult};
use infrarust_core::pipeline::types::{ConnectionIntent, HandshakeData};
use infrarust_protocol::version::ProtocolVersion;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

fn one_login_per(window: Duration) -> RateLimiterMiddleware {
    RateLimiterMiddleware::new(&RateLimitConfig {
        enabled: true,
        max_connections: 1,
        window,
        ..RateLimitConfig::default()
    })
}

async fn admits(limiter: &RateLimiterMiddleware, ip: &str) -> bool {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let stream = TcpStream::connect(addr).await.unwrap();
    let client_ip: IpAddr = ip.parse().unwrap();
    let mut ctx =
        ConnectionContext::new_for_test(stream, SocketAddr::new(client_ip, 50001), client_ip, addr);
    ctx.extensions.insert(HandshakeData {
        domain: "lobby.test".into(),
        raw_host: "lobby.test".into(),
        port: 25565,
        protocol_version: ProtocolVersion(767),
        intent: ConnectionIntent::Login,
        raw_packets: Vec::new(),
    });
    match limiter.process(&mut ctx).await.unwrap() {
        MiddlewareResult::Continue => true,
        MiddlewareResult::Reject(_) => false,
        _ => panic!("the rate limiter only continues or rejects"),
    }
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn only_the_first_refusal_warns_and_it_names_the_fix() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || writer.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);

    let limiter = one_login_per(Duration::from_secs(3600));
    assert!(admits(&limiter, "192.0.2.1").await);
    assert!(!admits(&limiter, "192.0.2.1").await);
    assert!(!admits(&limiter, "192.0.2.1").await);
    drop(guard);

    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert_eq!(logs.matches("WARN").count(), 1, "{logs}");
    assert!(logs.contains("[rate_limit]"), "{logs}");
    assert!(logs.contains("trusted_proxies"), "{logs}");
}

#[tokio::test]
async fn ipv6_addresses_in_one_64_share_a_budget() {
    let limiter = one_login_per(Duration::from_secs(3600));

    assert!(admits(&limiter, "2001:db8:1:1::1").await);
    assert!(!admits(&limiter, "2001:db8:1:1::2").await);
    assert!(admits(&limiter, "2001:db8:1:2::1").await);
    assert_eq!(limiter.tracked_keys(), 2);
}

#[tokio::test]
async fn rotating_addresses_in_one_64_hold_a_single_key() {
    let limiter = one_login_per(Duration::from_secs(3600));

    for host in 1..=100 {
        admits(&limiter, &format!("2001:db8:1:1::{host:x}")).await;
    }
    assert_eq!(limiter.tracked_keys(), 1);
}

#[tokio::test]
async fn an_ipv4_mapped_address_shares_its_ipv4_budget() {
    let limiter = one_login_per(Duration::from_secs(3600));

    assert!(admits(&limiter, "192.0.2.1").await);
    assert!(!admits(&limiter, "::ffff:192.0.2.1").await);
}

#[tokio::test]
async fn a_purge_forgets_expired_keys_and_keeps_live_ones() {
    let expired = one_login_per(Duration::from_millis(10));
    assert!(admits(&expired, "192.0.2.1").await);
    tokio::time::sleep(Duration::from_millis(100)).await;
    expired.purge();
    assert_eq!(expired.tracked_keys(), 0);

    let live = one_login_per(Duration::from_secs(3600));
    assert!(admits(&live, "192.0.2.1").await);
    live.purge();
    assert_eq!(live.tracked_keys(), 1);
    assert!(!admits(&live, "192.0.2.1").await);
}

#[tokio::test]
async fn the_purge_task_ends_with_the_shutdown_token() {
    let limiter = one_login_per(Duration::from_secs(10));
    let shutdown = CancellationToken::new();
    let task = limiter.start_purge_task(shutdown.clone()).unwrap();

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("the purge task stops on shutdown")
        .unwrap();
}

#[test]
fn a_disabled_limiter_starts_no_purge_task() {
    let limiter = RateLimiterMiddleware::new(&RateLimitConfig {
        enabled: false,
        ..RateLimitConfig::default()
    });
    assert!(limiter.start_purge_task(CancellationToken::new()).is_none());
}
