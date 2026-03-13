use infrarust_config::KeepaliveConfig;
use infrarust_transport::socket::*;
use std::net::SocketAddr;
use std::time::Duration;

#[test]
fn test_configure_listener_socket_binds() {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let socket = configure_listener_socket(addr, false).unwrap();
    let local = socket.local_addr().unwrap();
    assert!(local.as_socket().unwrap().port() > 0);
}

#[test]
fn test_reuseaddr_configured() {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let socket = configure_listener_socket(addr, false).unwrap();
    assert!(socket.reuse_address().unwrap());
}

#[cfg(target_os = "linux")]
#[test]
fn test_reuseport_configured() {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let socket = configure_listener_socket(addr, true).unwrap();
    assert!(socket.reuse_port().unwrap());
}

#[test]
fn test_nodelay_configured() {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener_socket = configure_listener_socket(addr, false).unwrap();
    let local_addr = listener_socket.local_addr().unwrap().as_socket().unwrap();

    // Create a stream socket and connect
    let stream_socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .unwrap();

    stream_socket.set_nonblocking(true).unwrap();
    let _ = stream_socket.connect(&local_addr.into());

    let keepalive = KeepaliveConfig {
        time: Duration::from_secs(30),
        interval: Duration::from_secs(10),
        retries: 3,
    };
    configure_stream_socket(&stream_socket, &keepalive).unwrap();
    assert!(stream_socket.nodelay().unwrap());
}

#[test]
fn test_keepalive_configured() {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener_socket = configure_listener_socket(addr, false).unwrap();
    let local_addr = listener_socket.local_addr().unwrap().as_socket().unwrap();

    let stream_socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .unwrap();

    stream_socket.set_nonblocking(true).unwrap();
    let _ = stream_socket.connect(&local_addr.into());

    let keepalive = KeepaliveConfig {
        time: Duration::from_secs(30),
        interval: Duration::from_secs(10),
        retries: 3,
    };
    configure_stream_socket(&stream_socket, &keepalive).unwrap();
    assert!(stream_socket.keepalive().unwrap());
}

#[tokio::test]
async fn test_into_tokio_listener() {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let socket = configure_listener_socket(addr, false).unwrap();
    let listener = into_tokio_listener(socket).unwrap();
    assert!(listener.local_addr().unwrap().port() > 0);
}
