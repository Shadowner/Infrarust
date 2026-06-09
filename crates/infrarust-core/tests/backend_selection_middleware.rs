//! Integration tests for `BackendSelectionMiddleware` (§15 of the LB spec).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use smallvec::SmallVec;

use infrarust_config::{ServerAddress, ServerConfig};
use infrarust_core::loadbalancer::{
    AddressConnectionCount, BackendCandidate, BackendHealthView, LeastConnections, LoadBalancer,
    select_backend_addresses,
};
use infrarust_core::middleware::backend_selection::{BackendSelectionMiddleware, BackendTargets};
use infrarust_core::pipeline::context::ConnectionContext;
use infrarust_core::pipeline::middleware::{Middleware, MiddlewareResult};
use infrarust_core::pipeline::types::RoutingData;

async fn make_test_ctx() -> ConnectionContext {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (_server, _) = listener.accept().await.unwrap();

    ConnectionContext::new_for_test(
        client,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12345),
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        addr,
    )
}

fn addr(host: &str) -> ServerAddress {
    ServerAddress {
        host: host.to_string(),
        port: 25565,
    }
}

fn server_config(addresses: &[&str]) -> ServerConfig {
    let list = addresses
        .iter()
        .map(|a| format!("\"{a}\""))
        .collect::<Vec<_>>()
        .join(", ");
    toml::from_str(&format!(
        r#"
        domains = ["test.mc"]
        addresses = [{list}]
    "#
    ))
    .expect("valid test config")
}

/// Per-address counts, settable from tests.
#[derive(Default)]
struct MockCounts {
    counts: Mutex<HashMap<ServerAddress, usize>>,
}

impl MockCounts {
    fn set(&self, a: &ServerAddress, n: usize) {
        self.counts.lock().unwrap().insert(a.clone(), n);
    }

    fn bump(&self, a: &ServerAddress) {
        *self.counts.lock().unwrap().entry(a.clone()).or_insert(0) += 1;
    }
}

impl AddressConnectionCount for MockCounts {
    fn active_connections_for_address(&self, a: &ServerAddress) -> usize {
        self.counts.lock().unwrap().get(a).copied().unwrap_or(0)
    }
}

/// Health view with per-address overrides (default: healthy, stable).
#[derive(Default)]
struct MockHealth {
    overrides: Mutex<HashMap<ServerAddress, (bool, Option<Instant>)>>,
}

impl MockHealth {
    fn set(&self, a: &ServerAddress, healthy: bool, since: Option<Instant>) {
        self.overrides
            .lock()
            .unwrap()
            .insert(a.clone(), (healthy, since));
    }
}

impl BackendHealthView for MockHealth {
    fn snapshot(&self, a: &ServerAddress) -> (bool, Option<Instant>) {
        self.overrides
            .lock()
            .unwrap()
            .get(a)
            .copied()
            .unwrap_or((true, None))
    }
}

/// LB that records the snapshots it receives and returns config order.
#[derive(Default)]
struct CapturingLb {
    calls: AtomicUsize,
    seen: Mutex<Vec<BackendCandidate>>,
}

impl LoadBalancer for CapturingLb {
    fn name(&self) -> &'static str {
        "capturing"
    }

    fn order<'a>(&self, candidates: &'a [BackendCandidate]) -> SmallVec<[&'a BackendCandidate; 4]> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.seen.lock().unwrap() = candidates.to_vec();
        candidates.iter().collect()
    }
}

fn insert_routing(
    ctx: &mut ConnectionContext,
    config: ServerConfig,
    load_balancer: Arc<dyn LoadBalancer>,
) {
    ctx.extensions.insert(RoutingData {
        server_config: Arc::new(config),
        config_id: "test".to_string(),
        load_balancer,
    });
}

#[tokio::test]
async fn test_middleware_no_routing_data_continues() {
    let middleware = BackendSelectionMiddleware::new(
        Arc::new(MockCounts::default()),
        Arc::new(MockHealth::default()),
    );
    let mut ctx = make_test_ctx().await;

    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));
    assert!(!ctx.extensions.contains::<BackendTargets>());
}

#[tokio::test]
async fn test_middleware_single_address_shortcut() {
    let lb = Arc::new(CapturingLb::default());
    let middleware = BackendSelectionMiddleware::new(
        Arc::new(MockCounts::default()),
        Arc::new(MockHealth::default()),
    );
    let mut ctx = make_test_ctx().await;
    insert_routing(&mut ctx, server_config(&["10.0.0.1:25565"]), lb.clone());

    middleware.process(&mut ctx).await.unwrap();

    let targets = ctx.extensions.get::<BackendTargets>().unwrap();
    assert_eq!(targets.addresses.as_slice(), [addr("10.0.0.1")]);
    assert_eq!(lb.calls.load(Ordering::SeqCst), 0, "LB must not be called");
}

#[tokio::test]
async fn test_middleware_builds_candidates() {
    let counts = Arc::new(MockCounts::default());
    let health = Arc::new(MockHealth::default());
    counts.set(&addr("10.0.0.1"), 7);
    let since = Instant::now();
    health.set(&addr("10.0.0.2"), false, None);
    health.set(&addr("10.0.0.1"), true, Some(since));

    let lb = Arc::new(CapturingLb::default());
    let middleware = BackendSelectionMiddleware::new(counts, health);
    let mut ctx = make_test_ctx().await;
    insert_routing(
        &mut ctx,
        server_config(&["10.0.0.1:25565", "10.0.0.2:25565"]),
        lb.clone(),
    );

    middleware.process(&mut ctx).await.unwrap();

    let seen = lb.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].address, addr("10.0.0.1"));
    assert_eq!(seen[0].active_connections, 7);
    assert!(seen[0].healthy);
    assert_eq!(seen[0].healthy_since, Some(since));
    assert_eq!(seen[1].address, addr("10.0.0.2"));
    assert_eq!(seen[1].active_connections, 0);
    assert!(!seen[1].healthy);
}

#[tokio::test]
async fn test_middleware_stores_ordered_targets() {
    // Real least_conn: the least loaded address must come first.
    let counts = Arc::new(MockCounts::default());
    counts.set(&addr("10.0.0.1"), 5);
    counts.set(&addr("10.0.0.2"), 1);
    counts.set(&addr("10.0.0.3"), 3);

    let middleware = BackendSelectionMiddleware::new(counts, Arc::new(MockHealth::default()));
    let mut ctx = make_test_ctx().await;
    insert_routing(
        &mut ctx,
        server_config(&["10.0.0.1:25565", "10.0.0.2:25565", "10.0.0.3:25565"]),
        Arc::new(LeastConnections::new(None)),
    );

    let result = middleware.process(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));

    let targets = ctx.extensions.get::<BackendTargets>().unwrap();
    assert_eq!(
        targets.addresses.as_slice(),
        [addr("10.0.0.2"), addr("10.0.0.3"), addr("10.0.0.1")]
    );
}

#[tokio::test]
async fn test_full_flow_least_conn_balances() {
    // Simulate 30 incoming sessions: each run of the middleware picks the
    // first target, which then "connects" (per-address count increments,
    // as the registry would after a successful connect). One LB instance
    // for the whole flow, like the DomainRouter holds per config.
    let counts = Arc::new(MockCounts::default());
    let health = Arc::new(MockHealth::default());
    let lb: Arc<dyn LoadBalancer> = Arc::new(LeastConnections::new(None));
    let middleware = BackendSelectionMiddleware::new(counts.clone(), health);

    let mut distribution: HashMap<String, usize> = HashMap::new();
    for _ in 0..30 {
        let mut ctx = make_test_ctx().await;
        insert_routing(
            &mut ctx,
            server_config(&["10.0.0.1:25565", "10.0.0.2:25565", "10.0.0.3:25565"]),
            lb.clone(),
        );
        middleware.process(&mut ctx).await.unwrap();

        let targets = ctx.extensions.get::<BackendTargets>().unwrap();
        let chosen = targets.addresses[0].clone();
        counts.bump(&chosen);
        *distribution.entry(chosen.host.clone()).or_insert(0) += 1;
    }

    assert_eq!(distribution.len(), 3);
    for (host, n) in &distribution {
        assert_eq!(*n, 10, "uneven distribution for {host}: {distribution:?}");
    }
}

#[test]
fn test_select_backend_addresses_shared_helper() {
    let counts = MockCounts::default();
    let health = MockHealth::default();
    counts.set(&addr("10.0.0.1"), 5);
    counts.set(&addr("10.0.0.2"), 1);
    health.set(&addr("10.0.0.2"), false, None);
    let lb = LeastConnections::new(None);

    let config = server_config(&["10.0.0.1:25565", "10.0.0.2:25565", "10.0.0.3:25565"]);
    let ordered = select_backend_addresses(&config, &lb, &counts, &health);
    assert_eq!(
        ordered.as_slice(),
        [addr("10.0.0.3"), addr("10.0.0.1"), addr("10.0.0.2")],
        "least loaded healthy first, unhealthy last"
    );

    // Single address: returned as-is, no balancing.
    let single = server_config(&["10.0.0.9:25565"]);
    let capturing = CapturingLb::default();
    let ordered = select_backend_addresses(&single, &capturing, &counts, &health);
    assert_eq!(ordered.as_slice(), [addr("10.0.0.9")]);
    assert_eq!(capturing.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn test_middleware_unhealthy_goes_last() {
    let counts = Arc::new(MockCounts::default());
    let health = Arc::new(MockHealth::default());
    // First address unhealthy despite being least loaded.
    health.set(&addr("10.0.0.1"), false, None);

    let middleware = BackendSelectionMiddleware::new(counts, health);
    let mut ctx = make_test_ctx().await;
    insert_routing(
        &mut ctx,
        server_config(&["10.0.0.1:25565", "10.0.0.2:25565"]),
        Arc::new(LeastConnections::new(None)),
    );

    middleware.process(&mut ctx).await.unwrap();

    let targets = ctx.extensions.get::<BackendTargets>().unwrap();
    assert_eq!(
        targets.addresses.as_slice(),
        [addr("10.0.0.2"), addr("10.0.0.1")],
        "unhealthy address must be the failover tail, never dropped"
    );
}
