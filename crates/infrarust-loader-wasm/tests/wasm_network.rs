#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/net.rs"]
mod net;
mod support;

use bytes::Bytes;
use infrarust_api::loader::PluginLoader;
use infrarust_api::types::{ProtocolVersion, RawPacket};
use infrarust_core::filter::codec_chain::build_codec_chains;
use net::{
    EchoServer, HttpServer, MuteServer, PROBE, SilentListener, UdpSink, enable_probe, free_port,
    network_toml,
};
use support::log_capture::LogCapture;
use support::{EnvOptions, load_enabled, loader_from_toml, make_env_with, stage};
use tracing::Level;
use tracing::instrument::WithSubscriber;

const DENIED: &str = "err PermissionDenied";

#[tokio::test(flavor = "multi_thread")]
async fn an_allowed_tcp_destination_echoes_and_a_refused_one_is_never_reached() {
    let allowed = EchoServer::start().await;
    let refused = SilentListener::start().await;
    let logs = LogCapture::at(Level::WARN);

    async {
        let probe =
            enable_probe(&["network"], &network_toml(&[allowed.addr.to_string()], "")).await;
        assert_eq!(
            probe.run(&format!("tcp {} hello", allowed.addr)).await,
            "ok hello"
        );
        assert_eq!(
            probe.run(&format!("tcp {} hello", refused.addr)).await,
            DENIED
        );
    }
    .with_subscriber(logs.clone())
    .await;

    assert_eq!(allowed.accepts_after_quiet().await, 1);
    assert!(refused.saw_no_connection().await);
    let denials = logs.matching("wasm plugin network access refused");
    assert_eq!(denials.len(), 1, "{:?}", logs.lines());
    assert!(
        denials[0].contains("plugin=net-probe")
            && denials[0].contains(&format!("destination={}", refused.addr))
            && denials[0].contains("reason=\"network allow-list\"")
            && denials[0].contains("kind=\"tcp-connect\""),
        "{denials:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn without_the_network_capability_the_allow_list_is_ignored() {
    let server = EchoServer::start().await;
    let http = HttpServer::start("never").await;
    let logs = LogCapture::at(Level::WARN);

    let caps = async {
        let probe = enable_probe(
            &[],
            &network_toml(
                &[
                    server.addr.to_string(),
                    http.addr.to_string(),
                    "localhost:*".to_owned(),
                ],
                "dns = true",
            ),
        )
        .await;
        assert_eq!(
            probe.run(&format!("tcp {} hello", server.addr)).await,
            DENIED
        );
        let fetched = probe.run(&format!("http http://{}/", http.addr)).await;
        assert!(fetched.contains("HttpRequestDenied"), "{fetched}");
        assert!(probe.run("dns localhost").await.starts_with("err "));
        assert!(probe.run("udp-bind 0.0.0.0:0").await.starts_with("err "));
        probe.run("caps").await
    }
    .with_subscriber(logs.clone())
    .await;

    assert_eq!(server.accepts_after_quiet().await, 0);
    assert_eq!(http.accepts_after_quiet().await, 0);
    assert!(!caps.split(',').any(|cap| cap == "network"), "{caps}");
    let ignored = logs.matching("wasm.network is ignored");
    assert_eq!(ignored.len(), 1, "{:?}", logs.lines());
    assert!(ignored[0].contains("`network` capability"), "{ignored:?}");
    let refused = logs.matching("missing capability `network`");
    assert!(!refused.is_empty(), "{:?}", logs.lines());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_allow_list_refuses_everything() {
    let server = EchoServer::start().await;
    let http = HttpServer::start("never").await;
    let logs = LogCapture::at(Level::WARN);

    let caps = async {
        let probe = enable_probe(&["network"], &network_toml(&[], "dns = true")).await;
        assert_eq!(
            probe.run(&format!("tcp {} hello", server.addr)).await,
            DENIED
        );
        let fetched = probe.run(&format!("http http://{}/", http.addr)).await;
        assert!(fetched.contains("HttpRequestDenied"), "{fetched}");
        assert!(probe.run("dns localhost").await.starts_with("err "));
        probe.run("caps").await
    }
    .with_subscriber(logs.clone())
    .await;

    assert_eq!(server.accepts_after_quiet().await, 0);
    assert_eq!(http.accepts_after_quiet().await, 0);
    assert!(caps.split(',').any(|cap| cap == "network"), "{caps}");
    let empty = logs.matching("empty allow list");
    assert_eq!(empty.len(), 1, "{:?}", logs.lines());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_cannot_listen_or_bind_a_fixed_port_by_default() {
    let port = free_port();
    let probe = enable_probe(&["network"], &network_toml(&["0.0.0.0/0:*".to_owned()], "")).await;
    assert_eq!(probe.run("listen 127.0.0.1:0").await, DENIED);
    assert_eq!(probe.run(&format!("listen 0.0.0.0:{port}")).await, DENIED);
    assert_eq!(
        probe.run(&format!("udp-bind 127.0.0.1:{port}")).await,
        DENIED
    );
    assert!(
        std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(),
        "the guest did not take the port"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exact_rule_lets_a_plugin_listen_on_that_address() {
    let port = free_port();
    let probe = enable_probe(
        &["network"],
        &network_toml(&[format!("127.0.0.1:{port}")], ""),
    )
    .await;
    assert_eq!(
        probe.run(&format!("listen 127.0.0.1:{port}")).await,
        format!("ok 127.0.0.1:{port}")
    );
    assert_eq!(probe.run("listen 127.0.0.1:0").await, DENIED);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hostname_rule_follows_what_the_host_resolves_and_its_ports() {
    let server = EchoServer::start().await;
    let port = server.addr.port();

    let probe = enable_probe(
        &["network"],
        &network_toml(&[format!("localhost:{port}")], ""),
    )
    .await;
    assert_eq!(
        probe.run(&format!("tcp localhost:{port} named")).await,
        "ok named"
    );
    assert_eq!(
        probe.run(&format!("tcp 127.0.0.1:{port} literal")).await,
        "ok literal"
    );
    assert_eq!(server.accepts_after_quiet().await, 2);

    let other = free_port();
    let mismatch = enable_probe(
        &["network"],
        &network_toml(&[format!("localhost:{other}")], ""),
    )
    .await;
    assert_eq!(
        mismatch.run(&format!("tcp localhost:{port} named")).await,
        DENIED
    );
    assert_eq!(server.accepts_after_quiet().await, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn udp_datagrams_reach_allowed_destinations_only() {
    let allowed = UdpSink::start().await;
    let refused = UdpSink::start().await;
    let probe = enable_probe(&["network"], &network_toml(&[allowed.addr.to_string()], "")).await;

    assert_eq!(
        probe.run(&format!("udp {} ping", allowed.addr)).await,
        "ok 4"
    );
    assert_eq!(allowed.received().await.as_deref(), Some("ping"));
    assert_eq!(
        probe.run(&format!("udp {} ping", refused.addr)).await,
        DENIED
    );
    assert_eq!(refused.received().await, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn dns_lookups_follow_the_dns_switch() {
    let off = enable_probe(&["network"], &network_toml(&["127.0.0.1:1".to_owned()], "")).await;
    assert!(off.run("dns localhost").await.starts_with("err "));
    let forced_off = enable_probe(
        &["network"],
        &network_toml(&["localhost:1".to_owned()], "dns = false"),
    )
    .await;
    assert!(forced_off.run("dns localhost").await.starts_with("err "));

    let on = enable_probe(&["network"], &network_toml(&["localhost:1".to_owned()], "")).await;
    let resolved = on.run("dns localhost").await;
    assert!(
        resolved.starts_with("ok ") && resolved.contains("127.0.0.1"),
        "{resolved}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_allowed_http_get_returns_status_and_body() {
    let server = HttpServer::start("hello from the host").await;
    let probe = enable_probe(&["network"], &network_toml(&[server.addr.to_string()], "")).await;
    assert_eq!(
        probe
            .run(&format!("http http://{}/greeting?x=1", server.addr))
            .await,
        "ok 200 hello from the host"
    );
    assert_eq!(server.accepts_after_quiet().await, 1);
    let requests = server.requests().await;
    assert!(
        requests[0].starts_with("GET /greeting?x=1 HTTP/1.1\r\n"),
        "{requests:?}"
    );
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains(&format!("host: {}\r\n", server.addr)),
        "{requests:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_http_get_to_an_allowed_hostname_is_resolved_by_the_host() {
    let server = HttpServer::start("named").await;
    let port = server.addr.port();
    let probe = enable_probe(
        &["network"],
        &network_toml(&[format!("localhost:{port}")], "dns = false"),
    )
    .await;
    assert_eq!(
        probe.run(&format!("http http://localhost:{port}/")).await,
        "ok 200 named"
    );
    assert_eq!(server.accepts_after_quiet().await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_http_get_is_denied_before_any_connection() {
    let allowed = HttpServer::start("allowed").await;
    let refused = HttpServer::start("refused").await;
    let logs = LogCapture::at(Level::WARN);

    async {
        let probe =
            enable_probe(&["network"], &network_toml(&[allowed.addr.to_string()], "")).await;
        let fetched = probe.run(&format!("http http://{}/", refused.addr)).await;
        assert!(
            fetched.starts_with("err ") && fetched.contains("HttpRequestDenied"),
            "{fetched}"
        );
        let fetched = probe
            .run(&format!("http http://localhost:{}/", allowed.addr.port()))
            .await;
        assert!(
            fetched.contains("HttpRequestDenied"),
            "an IP rule does not approve a name when dns is off: {fetched}"
        );
    }
    .with_subscriber(logs.clone())
    .await;

    assert_eq!(refused.accepts_after_quiet().await, 0);
    assert_eq!(allowed.accepts_after_quiet().await, 0);
    let denials = logs.matching("kind=\"http\"");
    assert!(
        denials.iter().any(
            |line| line.contains(&format!("destination={}", refused.addr))
                && line.contains("reason=\"network allow-list\"")
        ),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn http_false_refuses_every_request_but_leaves_sockets_alone() {
    let server = HttpServer::start("never").await;
    let echo = EchoServer::start().await;
    let logs = LogCapture::at(Level::WARN);
    async {
        let probe = enable_probe(
            &["network"],
            &network_toml(
                &[server.addr.to_string(), echo.addr.to_string()],
                "http = false",
            ),
        )
        .await;
        let fetched = probe.run(&format!("http http://{}/", server.addr)).await;
        assert!(fetched.contains("HttpRequestDenied"), "{fetched}");
        assert_eq!(
            probe.run(&format!("tcp {} socket", echo.addr)).await,
            "ok socket"
        );
    }
    .with_subscriber(logs.clone())
    .await;
    assert_eq!(server.accepts_after_quiet().await, 0);
    assert_eq!(
        logs.matching("reason=\"http = false\"").len(),
        1,
        "{:?}",
        logs.lines()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_http_request_waits_no_longer_than_host_call_timeout() {
    let server = MuteServer::start().await;
    let toml = format!(
        "[plugins.{PROBE}.wasm]\nhost_call_timeout = \"1s\"\n\n{}",
        network_toml(&[server.addr.to_string()], "")
    );
    let probe = enable_probe(&["network"], &toml).await;
    let started = std::time::Instant::now();
    let fetched = probe.run(&format!("http http://{}/", server.addr)).await;
    assert!(fetched.contains("ConnectionReadTimeout"), "{fetched}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rules_survive_a_trap_and_a_recovery() {
    let allowed = EchoServer::start().await;
    let refused = SilentListener::start().await;
    let probe = enable_probe(&["network"], &network_toml(&[allowed.addr.to_string()], "")).await;

    assert_eq!(
        probe.run(&format!("tcp {} before", allowed.addr)).await,
        "ok before"
    );
    probe.trap().await;
    assert_eq!(
        probe.run(&format!("tcp {} after", allowed.addr)).await,
        "ok after"
    );
    assert_eq!(
        probe.run(&format!("tcp {} after", refused.addr)).await,
        DENIED
    );
    assert_eq!(allowed.accepts_after_quiet().await, 2);
    assert!(refused.saw_no_connection().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn codec_filters_get_no_network_even_when_the_plugin_has_it() {
    let listener = SilentListener::start().await;
    let http = HttpServer::start("never").await;
    let (_tmp, plugins_dir) = stage("net-codec");
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions::default()
            .grant("net-codec", "codec-filter")
            .grant("net-codec", "network"),
    );
    let loader = loader_from_toml(&format!(
        "[plugins.net-codec.wasm.network]\nallow = [\"{}\", \"{}\"]\n",
        listener.addr, http.addr
    ));
    let logs = LogCapture::at(Level::INFO);

    let requests = [
        "echo filter-runs".to_owned(),
        format!("tcp {}", listener.addr),
        format!("http {}", http.addr),
    ];
    let outcomes = async {
        loader.discover(&plugins_dir).await.unwrap();
        let _plugin = load_enabled(&loader, &env.factory, "net-codec").await;
        assert!(!env.codec_registry.is_empty());
        let mut outcomes = Vec::new();
        for request in &requests {
            let (mut client, _server) = build_codec_chains(
                &env.codec_registry,
                ProtocolVersion::new(767),
                1,
                "127.0.0.1:1".parse().unwrap(),
                None,
            );
            let mut packet = RawPacket::new(0x05, Bytes::from(request.clone().into_bytes()));
            client.process(&mut packet);
            outcomes.push(String::from_utf8_lossy(&packet.data).into_owned());
        }
        outcomes
    }
    .with_subscriber(logs.clone())
    .await;

    assert!(listener.saw_no_connection().await);
    assert_eq!(http.accepts_after_quiet().await, 0);
    assert_eq!(
        outcomes,
        ["filter-runs", requests[1].as_str(), requests[2].as_str()],
        "the filter runs, but trapped on its first network call and the packets passed through untouched"
    );
    let traps = logs.matching("wasm codec filter trapped");
    assert_eq!(traps.len(), 2, "{:?}", logs.lines());
    assert!(
        traps
            .iter()
            .all(|line| line.contains("plugin=net-codec") && line.contains("op=\"filter\"")),
        "{traps:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn granted_capabilities_report_network_only_when_granted() {
    let with = enable_probe(&["network"], "").await;
    assert!(
        with.run("caps")
            .await
            .split(',')
            .any(|cap| cap == "network")
    );
    let without = enable_probe(&[], "").await;
    assert!(
        !without
            .run("caps")
            .await
            .split(',')
            .any(|cap| cap == "network")
    );
}
