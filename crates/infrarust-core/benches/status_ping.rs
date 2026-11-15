//! Status ping response on a fresh cache entry: what the proxy does between
//! reading the status request and writing the response frame, socket I/O
//! excluded.
//!
//! - `cached_hit` with `listeners = 0` serves the frame encoded once per cache
//!   entry.
//! - `cached_hit` with `listeners = 1` rebuilds the response on every ping so a
//!   `ProxyPingEvent` listener can edit it.
//! - `first_hit` is the encode a cache entry pays once, on its first ping.
//!
//! Run with: `cargo bench -p infrarust-core --bench status_ping`

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr};
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use base64::Engine;
use bytes::Bytes;
use divan::counter::BytesCount;
use divan::{Bencher, black_box};
use tokio::net::{TcpListener, TcpStream};

use infrarust_api::event::EventPriority;
use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::events::proxy::ProxyPingEvent;
use infrarust_config::{KeepaliveConfig, ServerConfig};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::loadbalancer::{BackendLoad, PassiveBackendHealth, build_load_balancer};
use infrarust_core::pipeline::context::ConnectionContext;
use infrarust_core::pipeline::types::{ConnectionIntent, HandshakeData, RoutingData};
use infrarust_core::registry::ConnectionRegistry;
use infrarust_core::status::relay_gate::RelayGate;
use infrarust_core::status::{
    FaviconCache, ServerPingResponse, StatusCache, StatusHandler, StatusRelayClient,
};
use infrarust_protocol::registry::build_default_registry;
use infrarust_protocol::version::ProtocolVersion;
use infrarust_transport::BackendConnector;

fn main() {
    divan::main();
}

const SERVER: &str = "survival";
const FAVICON_PNG_BYTES: usize = 9 * 1024;

fn favicon() -> String {
    let png: Vec<u8> = (0..FAVICON_PNG_BYTES)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add((i >> 8) as u8))
        .collect();
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}

fn backend_response() -> ServerPingResponse {
    serde_json::from_value(serde_json::json!({
        "version": {"name": "Paper 1.21.4", "protocol": 769},
        "players": {"max": 200, "online": 87, "sample": [
            {"name": "Notch", "id": "069a79f4-44e9-4726-a5be-fca90e38aaf5"},
            {"name": "jeb_", "id": "853c80ef-3c37-49fd-aa49-938b674adae6"},
            {"name": "Dinnerbone", "id": "61699b2e-d327-4a01-9f1e-0ea8c3f06bc6"}
        ]},
        "description": {
            "text": "",
            "extra": [
                {"text": "Survival", "color": "gold", "bold": true},
                {"text": " season 4\n", "color": "gray"},
                {"text": "play.example.net", "color": "aqua"}
            ]
        },
        "favicon": favicon(),
        "enforcesSecureChat": true
    }))
    .unwrap()
}

struct Fixture {
    handler: StatusHandler,
    cache: Arc<StatusCache>,
    ctx: ConnectionContext,
    sessions: ConnectionRegistry,
    _client: TcpStream,
    _runtime: tokio::runtime::Runtime,
}

fn fixture(listeners: usize) -> Fixture {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let bus = Arc::new(EventBusImpl::new());
    let bus_ref: &dyn EventBus = bus.as_ref();
    for _ in 0..listeners {
        bus_ref.subscribe::<ProxyPingEvent, _>(EventPriority::NORMAL, |event| {
            event.response.online_players += 1;
        });
    }

    let registry = Arc::new(build_default_registry());
    let cache = Arc::new(StatusCache::new(Duration::from_secs(3600)));
    cache.put(SERVER, backend_response(), Duration::from_millis(3), None);

    let favicons = runtime
        .block_on(FaviconCache::load_from_configs(&[], None))
        .unwrap();
    let handler = StatusHandler::new(
        StatusRelayClient::new(
            Arc::new(BackendConnector::new(
                Duration::from_secs(1),
                KeepaliveConfig::default(),
            )),
            Arc::clone(&registry),
            Duration::from_secs(1),
        ),
        Arc::clone(&cache),
        Arc::new(favicons),
        None,
        registry,
        None,
        bus,
        Arc::new(BackendLoad::new()),
        Arc::new(PassiveBackendHealth::new()),
        Arc::new(RelayGate::default()),
    );

    let (server, client, peer, local) = runtime.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = listener.local_addr().unwrap();
        let client = TcpStream::connect(local).await.unwrap();
        let (server, peer) = listener.accept().await.unwrap();
        (server, client, peer, local)
    });
    let mut ctx =
        ConnectionContext::new_for_test(server, peer, IpAddr::V4(Ipv4Addr::LOCALHOST), local);

    let config: Arc<ServerConfig> = Arc::new(
        toml::from_str("domains = [\"play.example.net\"]\naddresses = [\"127.0.0.1:25566\"]\n")
            .unwrap(),
    );
    ctx.extensions.insert(RoutingData {
        load_balancer: build_load_balancer(&config.balance_config()),
        server_config: config,
        config_id: SERVER.to_string(),
    });
    ctx.extensions.insert(HandshakeData {
        domain: "play.example.net".to_string(),
        raw_host: "play.example.net".to_string(),
        port: 25565,
        protocol_version: ProtocolVersion::V1_21,
        intent: ConnectionIntent::Status,
        raw_packets: Vec::new(),
    });

    Fixture {
        handler,
        cache,
        ctx,
        sessions: ConnectionRegistry::new(),
        _client: client,
        _runtime: runtime,
    }
}

fn poll_ready<F: Future>(future: F) -> F::Output {
    let mut cx = Context::from_waker(Waker::noop());
    match pin!(future).poll(&mut cx) {
        Poll::Ready(out) => out,
        Poll::Pending => panic!("the status frame was not built in one poll"),
    }
}

fn status_frame(fixture: &Fixture) -> Bytes {
    poll_ready(fixture.handler.status_frame(&fixture.ctx, &fixture.sessions)).unwrap()
}

#[divan::bench(args = [0, 1])]
fn cached_hit(bencher: Bencher, listeners: usize) {
    let fixture = fixture(listeners);
    let frame_len = status_frame(&fixture).len();
    bencher
        .counter(BytesCount::new(frame_len))
        .bench_local(|| black_box(status_frame(&fixture)));
}

#[divan::bench]
fn first_hit(bencher: Bencher) {
    let fixture = fixture(0);
    let frame_len = status_frame(&fixture).len();
    bencher
        .counter(BytesCount::new(frame_len))
        .with_inputs(|| {
            fixture
                .cache
                .put(SERVER, backend_response(), Duration::from_millis(3), None);
        })
        .bench_local_values(|()| black_box(status_frame(&fixture)));
}
