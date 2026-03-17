#![allow(clippy::unwrap_used, clippy::expect_used)]
use infrarust_config::ServerAddress;

#[test]
fn test_parse_host_port() {
    let addr: ServerAddress = "10.0.1.10:25565".parse().unwrap();
    assert_eq!(addr.host, "10.0.1.10");
    assert_eq!(addr.port, 25565);
}

#[test]
fn test_parse_host_only() {
    let addr: ServerAddress = "mc.example.com".parse().unwrap();
    assert_eq!(addr.host, "mc.example.com");
    assert_eq!(addr.port, 25565);
}

#[test]
fn test_parse_ipv6() {
    let addr: ServerAddress = "[::1]:25565".parse().unwrap();
    assert_eq!(addr.host, "::1");
    assert_eq!(addr.port, 25565);
}

#[test]
fn test_parse_empty_fails() {
    let result: Result<ServerAddress, _> = "".parse();
    assert!(result.is_err());
}

#[test]
fn test_display() {
    let addr = ServerAddress {
        host: "mc.example.com".to_string(),
        port: 25566,
    };
    assert_eq!(addr.to_string(), "mc.example.com:25566");
}
