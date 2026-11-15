#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Tests for the `TelemetryMiddleware` (NOT feature-gated — uses only tracing).

use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

use infrarust_config::{ProxyMode, TelemetryConfig, TraceClientIp, TracesConfig};
use infrarust_core::middleware::telemetry::{ConnectionSpan, TelemetryMiddleware};
use infrarust_core::pipeline::context::ConnectionContext;
use infrarust_core::pipeline::middleware::{Middleware, MiddlewareResult};
use infrarust_core::pipeline::types::{
    ConnectionIntent, HandshakeData, LoginData, LoginStartLayout, RoutingData,
};
use infrarust_core::telemetry::formatter::InfrarustFormatter;
use infrarust_protocol::version::ProtocolVersion;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;

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
    let middleware = TelemetryMiddleware::default();
    let mut ctx = make_test_ctx().await;

    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));
}

#[tokio::test]
async fn test_middleware_stores_span() {
    let middleware = TelemetryMiddleware::default();
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
    let middleware = TelemetryMiddleware::default();
    let mut ctx = make_test_ctx().await;

    let result = middleware.process(&mut ctx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_middleware_extracts_handshake_data() {
    let middleware = TelemetryMiddleware::default();
    let mut ctx = make_test_ctx().await;

    ctx.extensions.insert(HandshakeData {
        domain: "mc.example.com".to_string(),
        raw_host: "mc.example.com".to_string(),
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
    let middleware = TelemetryMiddleware::default();
    let mut ctx = make_test_ctx().await;

    let result = middleware.process(&mut ctx).await;
    assert!(result.is_ok());
    assert!(ctx.extensions.contains::<ConnectionSpan>());
}

#[tokio::test]
async fn test_middleware_extracts_login_data() {
    let middleware = TelemetryMiddleware::default();
    let mut ctx = make_test_ctx().await;

    ctx.extensions.insert(LoginData {
        username: "Notch".to_string(),
        player_uuid: None,
        profile_key: None,
        raw_payload: bytes::Bytes::new(),
        layout: LoginStartLayout::Declared,
    });

    ctx.extensions.insert(RoutingData {
        server_config: Arc::new(test_server_config(ProxyMode::Passthrough)),
        config_id: "survival".to_string(),
        load_balancer: Arc::new(infrarust_core::loadbalancer::FirstAvailable),
    });

    middleware.process(&mut ctx).await.unwrap();
    assert!(ctx.extensions.contains::<ConnectionSpan>());
}

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl io::Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Buffer {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

async fn span_line(middleware: TelemetryMiddleware, client_ip: &str) -> String {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .event_format(InfrarustFormatter::with(false, None))
            .with_writer(buffer.clone()),
    );
    let _default = tracing::subscriber::set_default(subscriber);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut ctx =
        ConnectionContext::new_for_test(stream, addr, client_ip.parse::<IpAddr>().unwrap(), addr);
    ctx.extensions.insert(LoginData {
        username: "Notch".to_string(),
        player_uuid: None,
        profile_key: None,
        raw_payload: bytes::Bytes::new(),
        layout: LoginStartLayout::Declared,
    });

    middleware.process(&mut ctx).await.unwrap();
    let span = ctx.extensions.remove::<ConnectionSpan>().unwrap();
    span.0.in_scope(|| tracing::info!("connected"));

    let bytes = buffer.0.lock().unwrap().clone();
    String::from_utf8(bytes).unwrap()
}

fn traces(client_ip: TraceClientIp, username: bool) -> TelemetryMiddleware {
    TelemetryMiddleware::new(&TracesConfig {
        client_ip,
        username,
        ..TracesConfig::default()
    })
}

#[tokio::test]
async fn the_default_span_truncates_the_client_ip_and_keeps_the_username() {
    let line = span_line(TelemetryMiddleware::default(), "203.0.113.77").await;
    assert!(line.contains("client.ip=203.0.113.0 "), "{line}");
    assert!(!line.contains("203.0.113.77"), "{line}");
    assert!(line.contains("player.username=Notch"), "{line}");
}

#[tokio::test]
async fn an_ipv6_client_is_truncated_to_its_48() {
    let line = span_line(TelemetryMiddleware::default(), "2001:db8:1:2:3:4:5:6").await;
    assert!(line.contains("client.ip=2001:db8:1:: "), "{line}");
}

#[tokio::test]
async fn a_mapped_ipv4_client_is_truncated_as_ipv4() {
    let line = span_line(TelemetryMiddleware::default(), "::ffff:203.0.113.77").await;
    assert!(line.contains("client.ip=203.0.113.0 "), "{line}");
}

#[tokio::test]
async fn the_full_address_is_only_recorded_when_asked_for() {
    let line = span_line(traces(TraceClientIp::Full, true), "203.0.113.77").await;
    assert!(line.contains("client.ip=203.0.113.77 "), "{line}");
}

#[tokio::test]
async fn both_fields_can_be_left_out() {
    let line = span_line(traces(TraceClientIp::Off, false), "203.0.113.77").await;
    assert!(!line.contains("client.ip"), "{line}");
    assert!(!line.contains("player.username"), "{line}");
    assert!(!line.contains("Notch"), "{line}");
    assert!(!line.contains("203.0.113"), "{line}");
}

#[test]
fn the_traces_section_defaults_to_a_truncated_ip_and_a_username() {
    let config: TelemetryConfig = toml::from_str("[traces]\nsampling_ratio = 0.5\n").unwrap();
    assert_eq!(config.traces.client_ip, TraceClientIp::Truncated);
    assert!(config.traces.username);
    assert_eq!(TracesConfig::default().client_ip, TraceClientIp::Truncated);
    assert!(TracesConfig::default().username);
}

#[test]
fn the_traces_section_reads_both_keys() {
    let config: TelemetryConfig =
        toml::from_str("[traces]\nclient_ip = \"off\"\nusername = false\n").unwrap();
    assert_eq!(config.traces.client_ip, TraceClientIp::Off);
    assert!(!config.traces.username);
    let full: TelemetryConfig = toml::from_str("[traces]\nclient_ip = \"full\"\n").unwrap();
    assert_eq!(full.traces.client_ip, TraceClientIp::Full);
}
