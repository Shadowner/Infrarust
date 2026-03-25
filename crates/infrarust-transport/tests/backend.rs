#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use infrarust_config::{KeepaliveConfig, ServerAddress};
use infrarust_transport::backend::BackendConnector;
use infrarust_transport::connection::ConnectionInfo;

fn test_connector() -> BackendConnector {
    BackendConnector::new(Duration::from_secs(5), KeepaliveConfig::default())
}

fn test_client_info(addr: SocketAddr) -> ConnectionInfo {
    ConnectionInfo {
        peer_addr: "127.0.0.1:12345".parse().unwrap(),
        real_ip: None,
        real_port: None,
        local_addr: addr,
        connected_at: tokio::time::Instant::now(),
    }
}

async fn spawn_echo_server() -> (SocketAddr, CancellationToken) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = CancellationToken::new();
    let t = token.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    let (mut stream, _) = result.unwrap();
                    tokio::spawn(async move {
                        let (mut r, mut w) = stream.split();
                        tokio::io::copy(&mut r, &mut w).await.ok();
                    });
                }
                () = t.cancelled() => break,
            }
        }
    });
    (addr, token)
}

#[tokio::test]
async fn test_connect_success() {
    let (addr, token) = spawn_echo_server().await;
    let connector = test_connector();

    let address = ServerAddress {
        host: addr.ip().to_string(),
        port: addr.port(),
    };

    let conn = connector
        .connect("test", &[address], None, false, &test_client_info(addr))
        .await
        .unwrap();

    assert_eq!(conn.remote_addr(), addr);
    token.cancel();
}

#[tokio::test]
async fn test_connect_timeout() {
    let connector = BackendConnector::new(Duration::from_millis(100), KeepaliveConfig::default());

    // Use a non-routable address
    let address = ServerAddress {
        host: "10.255.255.1".to_string(),
        port: 1,
    };

    let info = test_client_info("127.0.0.1:25565".parse().unwrap());
    let result = connector
        .connect("test", &[address], None, false, &info)
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_failover_to_second_address() {
    let (addr, token) = spawn_echo_server().await;
    let connector = BackendConnector::new(Duration::from_millis(500), KeepaliveConfig::default());

    let bad_address = ServerAddress {
        host: "127.0.0.1".to_string(),
        port: 1, // No server here
    };
    let good_address = ServerAddress {
        host: addr.ip().to_string(),
        port: addr.port(),
    };

    let conn = connector
        .connect(
            "test",
            &[bad_address, good_address],
            None,
            false,
            &test_client_info(addr),
        )
        .await
        .unwrap();

    assert_eq!(conn.remote_addr(), addr);
    token.cancel();
}

#[tokio::test]
async fn test_all_addresses_fail() {
    let connector = BackendConnector::new(Duration::from_millis(100), KeepaliveConfig::default());

    let addresses = vec![
        ServerAddress {
            host: "127.0.0.1".to_string(),
            port: 1,
        },
        ServerAddress {
            host: "127.0.0.1".to_string(),
            port: 2,
        },
    ];

    let info = test_client_info("127.0.0.1:25565".parse().unwrap());
    let result = connector
        .connect("test-server", &addresses, None, false, &info)
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_nodelay_enabled() {
    let (addr, token) = spawn_echo_server().await;
    let connector = test_connector();

    let address = ServerAddress {
        host: addr.ip().to_string(),
        port: addr.port(),
    };

    let conn = connector
        .connect("test", &[address], None, false, &test_client_info(addr))
        .await
        .unwrap();

    assert!(conn.stream().nodelay().unwrap());
    token.cancel();
}
