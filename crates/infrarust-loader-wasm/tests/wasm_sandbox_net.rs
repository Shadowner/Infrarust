#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/net.rs"]
mod net;
mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use net::{EchoServer, HttpServer, SilentListener, enable_probe, free_port, network_toml};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const DENIED: &str = "err PermissionDenied";

#[tokio::test(flavor = "multi_thread")]
async fn ipv4_mapped_ipv6_addresses_never_reach_a_destination() {
    let echo = EchoServer::start().await;
    let mapped = format!("[::ffff:127.0.0.1]:{}", echo.addr.port());

    let v6_all = enable_probe(&["network"], &network_toml(&["[::/0]:*".to_owned()], "")).await;
    assert!(
        v6_all.run(&format!("tcp {mapped} x")).await.starts_with("err "),
        "an IPv6-all rule must not reach an IPv4 address in mapped form"
    );

    let v4_all = enable_probe(&["network"], &network_toml(&["0.0.0.0/0:*".to_owned()], "")).await;
    assert!(
        v4_all.run(&format!("tcp {mapped} x")).await.starts_with("err "),
        "wasmtime rejects an IPv4-mapped IPv6 literal before the allow-list, so it reaches nothing"
    );
    assert_eq!(
        echo.accepts_after_quiet().await,
        0,
        "no connection reached the echo server via a mapped address"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_unspecified_address_is_not_a_valid_connect_destination() {
    let port = free_port();
    let probe = enable_probe(&["network"], &network_toml(&["0.0.0.0/0:*".to_owned()], "")).await;
    let v4 = probe.run(&format!("tcp 0.0.0.0:{port} x")).await;
    assert!(v4.starts_with("err "), "0.0.0.0 is not a connect target: {v4}");
    let v6 = probe.run(&format!("tcp [::]:{port} x")).await;
    assert!(v6.starts_with("err "), "[::] is not a connect target: {v6}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_local_address_is_refused_by_a_narrow_allow_list() {
    let echo = EchoServer::start().await;
    let probe = enable_probe(
        &["network"],
        &network_toml(&[echo.addr.to_string()], ""),
    )
    .await;
    assert_eq!(
        probe.run("tcp 169.254.169.254:80 x").await,
        DENIED,
        "a link-local address no rule covers must be refused"
    );
    assert_eq!(probe.run("tcp [fe80::1]:80 x").await, DENIED);
    assert_eq!(echo.accepts_after_quiet().await, 0);
}

struct RedirectServer {
    addr: std::net::SocketAddr,
    accepts: Arc<AtomicUsize>,
}

impl RedirectServer {
    async fn start(location: String) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&accepts);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                let location = location.clone();
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        match stream.read(&mut byte).await {
                            Ok(1) => head.push(byte[0]),
                            _ => return,
                        }
                    }
                    let response = format!(
                        "HTTP/1.1 302 Found\r\nLocation: {location}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        Self { addr, accepts }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_http_redirect_to_a_denied_host_is_not_followed() {
    let denied = SilentListener::start().await;
    let redirect = RedirectServer::start(format!("http://{}/secret", denied.addr)).await;
    let probe = enable_probe(
        &["network"],
        &network_toml(&[redirect.addr.to_string()], ""),
    )
    .await;

    let outcome = probe.run(&format!("http http://{}/", redirect.addr)).await;
    assert!(
        outcome.contains("302"),
        "the guest sees the redirect response, not a followed request: {outcome}"
    );
    assert!(
        denied.saw_no_connection().await,
        "wasi-http must not follow a redirect to a denied host"
    );
    assert_eq!(redirect.accepts.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_http_response_body_streams_to_the_guest() {
    let body: &'static str = Box::leak(vec![b'a'; 512 * 1024].into_iter().map(char::from).collect::<String>().into_boxed_str());
    let server = HttpServer::start(body).await;
    let probe = enable_probe(
        &["network"],
        &network_toml(&[server.addr.to_string()], ""),
    )
    .await;
    let outcome = probe.run(&format!("http http://{}/", server.addr)).await;
    assert!(
        outcome.starts_with("ok 200") && outcome.len() >= 512 * 1024,
        "the guest streamed the full body without the host buffering it unbounded (len {})",
        outcome.len()
    );
}
