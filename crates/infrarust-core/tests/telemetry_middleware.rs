#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Tests for the `TelemetryMiddleware` (NOT feature-gated — uses only tracing).

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use infrarust_config::ProxyMode;
use infrarust_core::middleware::telemetry::{ConnectionSpan, TelemetryMiddleware};
use infrarust_core::pipeline::context::ConnectionContext;
use infrarust_core::pipeline::middleware::{Middleware, MiddlewareResult};
use infrarust_core::pipeline::types::{ConnectionIntent, HandshakeData, LoginData, RoutingData};
use infrarust_protocol::version::ProtocolVersion;

/// Helper to create a minimal `ServerConfig` for tests.
fn test_server_config(proxy_mode: ProxyMode) -> infrarust_config::ServerConfig {
    let toml_str = format!(
        r#"
        domains = ["test.example.com"]
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "{proxy_mode:?}"
        max_players = 20
        "#
    );
    // ProxyMode serializes as snake_case
    let toml_str = toml_str.replace("Passthrough", "passthrough");
    toml::from_str(&toml_str).expect("valid test server config")
}

/// Helper to create a test `ConnectionContext` with a dummy TCP stream.
async fn make_test_ctx() -> ConnectionContext {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (_accepted, _) = listener.accept().await.unwrap();

    ConnectionContext::new_for_test(stream, addr, IpAddr::V4(Ipv4Addr::LOCALHOST), addr)
}

#[tokio::test]
async fn test_middleware_always_continues() {
    let middleware = TelemetryMiddleware;
    let mut ctx = make_test_ctx().await;

    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));
}

#[tokio::test]
async fn test_middleware_stores_span() {
    let middleware = TelemetryMiddleware;
    let mut ctx = make_test_ctx().await;

    middleware.process(&mut ctx).await.unwrap();

    assert!(
        ctx.extensions.contains::<ConnectionSpan>(),
        "ConnectionSpan should be in extensions after process()"
    );
}

#[tokio::test]
async fn test_middleware_works_without_otel() {
    // No OTel subscriber installed — should not panic
    let middleware = TelemetryMiddleware;
    let mut ctx = make_test_ctx().await;

    let result = middleware.process(&mut ctx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_middleware_extracts_handshake_data() {
    let middleware = TelemetryMiddleware;
    let mut ctx = make_test_ctx().await;

    ctx.extensions.insert(HandshakeData {
        domain: "mc.example.com".to_string(),
        port: 25565,
        protocol_version: ProtocolVersion(767),
        intent: ConnectionIntent::Login,
        raw_packets: vec![],
    });

    middleware.process(&mut ctx).await.unwrap();

    // Span was created (we can't inspect fields, but we can verify it exists)
    assert!(ctx.extensions.contains::<ConnectionSpan>());
}

#[tokio::test]
async fn test_middleware_handles_empty_extensions() {
    // No HandshakeData, RoutingData, or LoginData — should not panic
    let middleware = TelemetryMiddleware;
    let mut ctx = make_test_ctx().await;

    let result = middleware.process(&mut ctx).await;
    assert!(result.is_ok());
    assert!(ctx.extensions.contains::<ConnectionSpan>());
}

#[tokio::test]
async fn test_middleware_extracts_login_data() {
    let middleware = TelemetryMiddleware;
    let mut ctx = make_test_ctx().await;

    ctx.extensions.insert(LoginData {
        username: "Notch".to_string(),
        player_uuid: None,
    });

    ctx.extensions.insert(RoutingData {
        server_config: Arc::new(test_server_config(ProxyMode::Passthrough)),
        config_id: "survival".to_string(),
    });

    middleware.process(&mut ctx).await.unwrap();
    assert!(ctx.extensions.contains::<ConnectionSpan>());
}
