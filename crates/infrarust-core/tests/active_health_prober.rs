#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The prober is driven by per-server settings, so the proxy-wide
//! `[active_health] enabled` flag must not decide whether it runs at all.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use infrarust_config::ProxyConfig;
use infrarust_core::server::ProxyServer;

#[tokio::test]
async fn a_server_opting_into_probing_is_probed_with_the_proxy_block_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let servers_dir = dir.path().join("servers");
    std::fs::create_dir(&servers_dir).unwrap();

    let backend = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = backend.local_addr().unwrap().port();
    std::fs::write(
        servers_dir.join("probed.toml"),
        format!(
            "domains = [\"probed.test\"]\naddresses = [\"127.0.0.1:{port}\"]\n\
             [active_health]\nprobe_healthy = true\ninterval = \"0s\"\n"
        ),
    )
    .unwrap();

    let config: ProxyConfig = toml::from_str(&format!(
        "bind = \"127.0.0.1:0\"\nservers_dir = \"{}\"\n[active_health]\nenabled = false\n",
        servers_dir.display()
    ))
    .unwrap();

    let shutdown = CancellationToken::new();
    let proxy = Arc::new(
        ProxyServer::new(config, dir.path().join("infrarust.toml"), shutdown.clone())
            .await
            .unwrap(),
    );
    tokio::spawn(Arc::clone(&proxy).run());

    let probed = tokio::time::timeout(Duration::from_secs(5), backend.accept()).await;
    shutdown.cancel();

    assert!(
        probed.is_ok(),
        "a server enabling active health must be probed even when the proxy-wide block is off"
    );
}
