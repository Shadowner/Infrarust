use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

use infrarust_transport::proxy_protocol::*;

#[tokio::test]
async fn test_decode_v1_tcp4() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut server_stream, _) = listener.accept().await.unwrap();

    // Send a v1 proxy protocol header
    let header = b"PROXY TCP4 192.168.1.1 10.0.0.1 12345 25565\r\n";
    client.write_all(header).await.unwrap();

    let (info, consumed) = decode_proxy_protocol(&mut server_stream).await.unwrap();

    assert_eq!(info.version, ProxyProtocolVersion::V1);
    assert_eq!(info.source_addr, "192.168.1.1:12345".parse().unwrap());
    assert_eq!(info.dest_addr, "10.0.0.1:25565".parse().unwrap());
    assert_eq!(consumed, header.len());
}

#[tokio::test]
async fn test_decode_v2_tcp4() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut server_stream, _) = listener.accept().await.unwrap();

    // Build a v2 header using ppp
    let addresses = ppp::v2::IPv4::new([192, 168, 1, 1], [10, 0, 0, 1], 12345, 25565);
    let version_command = ppp::v2::Version::Two as u8 | ppp::v2::Command::Proxy as u8;
    let header_bytes =
        ppp::v2::Builder::with_addresses(version_command, ppp::v2::Protocol::Stream, addresses)
            .build()
            .unwrap();

    client.write_all(&header_bytes).await.unwrap();

    let (info, consumed) = decode_proxy_protocol(&mut server_stream).await.unwrap();

    assert_eq!(info.version, ProxyProtocolVersion::V2);
    assert_eq!(info.source_addr, "192.168.1.1:12345".parse().unwrap());
    assert_eq!(info.dest_addr, "10.0.0.1:25565".parse().unwrap());
    assert_eq!(consumed, header_bytes.len());
}

#[tokio::test]
async fn test_decode_v2_tcp6() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut server_stream, _) = listener.accept().await.unwrap();

    let src: std::net::Ipv6Addr = "::1".parse().unwrap();
    let dst: std::net::Ipv6Addr = "fe80::1".parse().unwrap();
    let addresses = ppp::v2::IPv6::new(src.octets(), dst.octets(), 54321, 25565);
    let version_command = ppp::v2::Version::Two as u8 | ppp::v2::Command::Proxy as u8;
    let header_bytes =
        ppp::v2::Builder::with_addresses(version_command, ppp::v2::Protocol::Stream, addresses)
            .build()
            .unwrap();

    client.write_all(&header_bytes).await.unwrap();

    let (info, _consumed) = decode_proxy_protocol(&mut server_stream).await.unwrap();

    assert_eq!(info.version, ProxyProtocolVersion::V2);
    assert_eq!(info.source_addr, "[::1]:54321".parse().unwrap());
    assert_eq!(info.dest_addr, "[fe80::1]:25565".parse().unwrap());
}

#[tokio::test]
async fn test_encode_v2_tcp4() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut server_stream, _) = listener.accept().await.unwrap();

    let info = infrarust_transport::ConnectionInfo {
        peer_addr: "192.168.1.1:12345".parse().unwrap(),
        real_ip: None,
        real_port: None,
        local_addr: "10.0.0.1:25565".parse().unwrap(),
        connected_at: tokio::time::Instant::now(),
    };

    encode_proxy_protocol_v2(&mut client, &info).await.unwrap();

    // Read what was sent and parse it back
    let (decoded, _) = decode_proxy_protocol(&mut server_stream).await.unwrap();
    assert_eq!(decoded.version, ProxyProtocolVersion::V2);
    assert_eq!(decoded.source_addr, "192.168.1.1:12345".parse().unwrap());
}

#[tokio::test]
async fn test_no_proxy_protocol() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut server_stream, _) = listener.accept().await.unwrap();

    // Send garbage data
    client
        .write_all(b"NOT A PROXY PROTOCOL HEADER AT ALL!!")
        .await
        .unwrap();

    let result = decode_proxy_protocol(&mut server_stream).await;
    assert!(result.is_err());
}
