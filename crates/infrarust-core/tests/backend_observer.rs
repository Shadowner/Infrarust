//! `ConnectAttemptObserver` wiring: `PassiveBackendHealth` must receive
//! per-address connect outcomes through the transport observer during
//! failover.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use infrarust_config::{KeepaliveConfig, ServerAddress};
use infrarust_core::loadbalancer::{BackendHealthView, BackendState, PassiveBackendHealth};
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
    assert_ne!(
        health.snapshot(&dead).state,
        BackendState::Healthy,
        "failed attempt must mark the dead address unhealthy"
    );
    assert_eq!(health.snapshot(&live).state, BackendState::Healthy);
}

#[tokio::test]
async fn max_attempts_bounds_the_failover_walk() {
    let health = Arc::new(PassiveBackendHealth::new());
    let connector = BackendConnector::new(Duration::from_millis(50), KeepaliveConfig::default())
        .with_max_attempts(2)
        .with_observer(Arc::clone(&health) as _);

    let dead: Vec<ServerAddress> = ["127.0.0.1:1", "127.0.0.1:2", "127.0.0.1:3"]
        .iter()
        .map(|a| a.parse().unwrap())
        .collect();
    let peer = "127.0.0.1:1".parse().unwrap();
    let info = ConnectionInfo {
        peer_addr: peer,
        real_ip: None,
        real_port: None,
        local_addr: peer,
        connected_at: tokio::time::Instant::now(),
    };

    assert!(
        connector
            .connect("test", &dead, None, false, &info)
            .await
            .is_err()
    );
    assert_eq!(
        health.snapshot(&dead[2]).state,
        BackendState::Healthy,
        "the third address must never have been dialled"
    );
}
