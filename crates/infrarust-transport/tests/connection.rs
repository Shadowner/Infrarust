use std::net::IpAddr;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

use infrarust_transport::connection::ClientConnection;
use infrarust_transport::proxy_protocol::{ProxyProtocolInfo, ProxyProtocolVersion};

#[tokio::test]
async fn test_client_addr_without_proxy() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let _client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (server_stream, peer_addr) = listener.accept().await.unwrap();

    let conn = ClientConnection::new(server_stream, peer_addr, addr);
    assert_eq!(conn.client_addr(), peer_addr.ip());
}

#[tokio::test]
async fn test_client_addr_with_proxy() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let _client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (server_stream, peer_addr) = listener.accept().await.unwrap();

    let real_ip: IpAddr = "192.168.1.100".parse().unwrap();
    let info = ProxyProtocolInfo {
        source_addr: "192.168.1.100:12345".parse().unwrap(),
        dest_addr: "10.0.0.1:25565".parse().unwrap(),
        version: ProxyProtocolVersion::V2,
    };

    let conn = ClientConnection::new(server_stream, peer_addr, addr).with_proxy_protocol(&info);
    assert_eq!(conn.client_addr(), real_ip);
    assert_eq!(conn.real_ip(), Some(real_ip));
}

#[tokio::test]
async fn test_peek_first_bytes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (server_stream, peer_addr) = listener.accept().await.unwrap();

    // Send some data
    client.write_all(b"hello").await.unwrap();

    let mut conn = ClientConnection::new(server_stream, peer_addr, addr);
    let peeked = conn.peek(3).await.unwrap();
    assert_eq!(peeked, b"hel");
}

#[tokio::test]
async fn test_into_parts_returns_buffered() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (server_stream, peer_addr) = listener.accept().await.unwrap();

    client.write_all(b"hello world").await.unwrap();

    let mut conn = ClientConnection::new(server_stream, peer_addr, addr);
    conn.peek(5).await.unwrap();

    let (_stream, buffered, info) = conn.into_parts();
    assert_eq!(&buffered[..], b"hello");
    assert_eq!(info.peer_addr, peer_addr);
}
