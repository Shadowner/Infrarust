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

    let limiter = RateLimiterMiddleware::new(&RateLimitConfig {
        enabled: true,
        max_connections: 1,
        window: Duration::from_secs(3600),
        ..RateLimitConfig::default()
    });
    assert!(admits(&limiter, "192.0.2.1").await);
    assert!(!admits(&limiter, "192.0.2.1").await);
    assert!(!admits(&limiter, "192.0.2.1").await);
    drop(guard);

    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert_eq!(logs.matches("WARN").count(), 1, "{logs}");
    assert!(logs.contains("[rate_limit]"), "{logs}");
    assert!(logs.contains("trusted_proxies"), "{logs}");
}
