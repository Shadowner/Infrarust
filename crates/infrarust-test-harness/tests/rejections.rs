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
