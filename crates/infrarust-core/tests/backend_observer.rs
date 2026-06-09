//! `ConnectAttemptObserver` wiring: `PassiveBackendHealth` must receive
//! per-address connect outcomes through the transport observer during
//! failover.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use infrarust_config::{KeepaliveConfig, ServerAddress};
use infrarust_core::loadbalancer::{BackendHealthView, PassiveBackendHealth};
use infrarust_transport::{BackendConnector, ConnectionInfo};

#[tokio::test]
async fn observer_records_failover_outcomes() {
    // Dead address: bind then drop, so the port refuses connections.
    let dead = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        ServerAddress {
            host: "127.0.0.1".to_string(),
            port,
        }
    };
    let live_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let live = ServerAddress {
        host: "127.0.0.1".to_string(),
        port: live_listener.local_addr().unwrap().port(),
    };

    let health = Arc::new(PassiveBackendHealth::with_threshold(1));
    let connector = BackendConnector::new(Duration::from_secs(2), KeepaliveConfig::default())
        .with_observer(Arc::clone(&health) as _);

    let info = ConnectionInfo {
        peer_addr: "127.0.0.1:1".parse().unwrap(),
        real_ip: None,
        real_port: None,
        local_addr: "127.0.0.1:1".parse().unwrap(),
        connected_at: tokio::time::Instant::now(),
    };
    let conn = connector
        .connect("test", &[dead.clone(), live.clone()], None, false, &info)
        .await
        .unwrap();

    assert_eq!(
        conn.server_address(),
        &live,
        "failover must reach the live address"
    );
    assert!(
        !health.snapshot(&dead).0,
        "failed attempt must mark the dead address unhealthy"
    );
    assert!(health.snapshot(&live).0);
}
