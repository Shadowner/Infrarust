#![allow(clippy::unwrap_used, clippy::expect_used)]

use infrarust_protocol::{SHandshake, SStatusRequest, VarInt};
use infrarust_test_harness::framing::FramedConn;
use infrarust_test_harness::{
    ConnectionState, FakeBackend, HarnessError, HarnessResult, ProtocolVersion, ServerSpec,
    TestProxy, TestProxyBuilder, wire,
};
use toml::Table;

const VERSION: ProtocolVersion = ProtocolVersion::V1_21;

fn block_localhost() -> toml::Value {
    toml::from_str::<Table>("blacklist = [\"127.0.0.1/32\"]")
        .unwrap()
        .into()
}

async fn start(builder: TestProxyBuilder) -> (TestProxy, FakeBackend) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = builder
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();
    (proxy, backend)
}

async fn status_reply(proxy: &TestProxy, domain: &str) -> HarnessResult<bool> {
    let mut conn = FramedConn::connect(proxy.addr()).await?;
    let handshake = SHandshake {
        protocol_version: VarInt(VERSION.0),
        server_address: domain.to_string(),
        server_port: proxy.addr().port(),
        next_state: ConnectionState::Status,
        transfer: false,
    };
    conn.write_frames(&[
        wire::encode(&handshake, VERSION)?,
        wire::encode(&SStatusRequest, VERSION)?,
    ])
    .await?;
    match conn.read_frame().await {
        Ok(frame) => Ok(frame.is_some()),
        Err(HarnessError::Io(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

fn closed_without_answer<T: std::fmt::Debug>(result: HarnessResult<T>) {
    match result {
        Err(HarnessError::Closed(_) | HarnessError::Io(_)) => {}
        other => panic!("expected the proxy to close the connection silently, got {other:?}"),
    }
}

#[tokio::test]
async fn a_status_ping_turned_away_by_the_global_ip_filter_gets_no_reply() {
    let (proxy, _backend) = start(TestProxy::builder().patch_config(|table| {
        table.insert("ip_filter".into(), block_localhost());
    }))
    .await;

    assert!(
        !status_reply(&proxy, proxy.domain("lobby").unwrap())
            .await
            .unwrap()
    );

    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_login_turned_away_by_the_global_ip_filter_is_closed_without_a_kick() {
    let (proxy, _backend) = start(TestProxy::builder().patch_config(|table| {
        table.insert("ip_filter".into(), block_localhost());
    }))
    .await;

    closed_without_answer(proxy.client(VERSION).login("Eve").await);

    proxy.shutdown().await.unwrap();
}

fn drop_unknown_domains(table: &mut Table) {
    table.insert("unknown_domain_behavior".into(), "drop".into());
}

#[tokio::test]
async fn the_drop_behavior_closes_an_unknown_domain_login_without_a_kick() {
    let (proxy, _backend) = start(TestProxy::builder().patch_config(drop_unknown_domains)).await;

    closed_without_answer(
        proxy
            .client(VERSION)
            .domain("nowhere.test")
            .login("Bob")
            .await,
    );
    assert!(!status_reply(&proxy, "nowhere.test").await.unwrap());

    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_drop_behavior_still_kicks_a_rate_limited_login() {
    let (proxy, _backend) = start(TestProxy::builder().patch_config(|table| {
        drop_unknown_domains(table);
        let rate_limit: Table = toml::from_str(
            "enabled = true\nmax_connections = 1\nwindow = \"1h\"\nstatus_max = 100\nstatus_window = \"1h\"",
        )
        .unwrap();
        table.insert("rate_limit".into(), rate_limit.into());
    }))
    .await;

    let _alice = proxy
        .client(VERSION)
        .login("Alice")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let kick = proxy
        .client(VERSION)
        .login("Alice")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(kick.text, "Rate limit exceeded");

    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_drop_behavior_still_kicks_a_login_blocked_by_a_server_ip_filter() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("guarded")
                .backend(backend.addr())
                .patch(|table| {
                    table.insert("ip_filter".into(), block_localhost());
                }),
        )
        .patch_config(drop_unknown_domains)
        .start()
        .await
        .unwrap();

    let kick = proxy
        .client(VERSION)
        .login("Eve")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(kick.text, "IP 127.0.0.1 is not allowed on this server");

    proxy.shutdown().await.unwrap();
}
