#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::event::{BoxFuture, EventPriority};
use infrarust_api::events::lifecycle::PostLoginEvent;
use infrarust_api::filter::{
    CodecFilterFactory, CodecFilterInstance, CodecSessionInit, CodecVerdict, FilterMetadata,
    FilterPriority, FilterVerdict, FrameOutput, TransportContext, TransportFilter,
};
use infrarust_api::types::RawPacket;
use infrarust_config::ProxyMode;
use infrarust_test_harness::legacy::LEGACY_PROTOCOL;
use infrarust_test_harness::{
    DEFAULT_TIMEOUT, EventKind, FakeBackend, FakeClient, FakeLegacyBackend, HarnessError,
    LegacyPing, ProtocolVersion, Recorder, ScriptedPlugin, ServerSpec, TestProxy, TestProxyBuilder,
};
use serde_json::json;
use tokio::sync::{Notify, oneshot, watch};
use toml::{Table, Value};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt;

const T: Duration = DEFAULT_TIMEOUT;
const VERSION: ProtocolVersion = ProtocolVersion::V1_21;
const OWNER: &str = "gatekeeper";
const WATCH: &str = "watch";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hook {
    Accept,
    Close,
}

#[derive(Debug, Clone)]
struct Seen {
    hook: Hook,
    filter: &'static str,
    id: u64,
    remote: SocketAddr,
    real_ip: Option<IpAddr>,
}

#[derive(Clone)]
struct Log(Arc<watch::Sender<Vec<Seen>>>);

impl Log {
    fn new() -> Self {
        Self(Arc::new(watch::channel(Vec::new()).0))
    }

    fn record(&self, hook: Hook, filter: &'static str, ctx: &TransportContext) {
        self.0.send_modify(|seen| {
            seen.push(Seen {
                hook,
                filter,
                id: ctx.connection_id,
                remote: ctx.remote_addr,
                real_ip: ctx.real_ip,
            });
        });
    }

    fn seen(&self) -> Vec<Seen> {
        self.0.borrow().clone()
    }

    fn of(&self, hook: Hook, filter: &str) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|seen| seen.hook == hook && seen.filter == filter)
            .collect()
    }

    async fn until(&self, what: &str, mut done: impl FnMut(&[Seen]) -> bool) {
        let mut seen = self.0.subscribe();
        let waited = tokio::time::timeout(T, seen.wait_for(|log| done(log))).await;
        assert!(
            waited.is_ok(),
            "timed out waiting for {what}: {:#?}",
            self.seen()
        );
    }

    async fn closes(&self, filter: &'static str, count: usize) {
        self.until(&format!("{count} on_close of {filter}"), |log| {
            log.iter()
                .filter(|seen| seen.hook == Hook::Close && seen.filter == filter)
                .count()
                >= count
        })
        .await;
    }

    fn assert_paired(&self, filter: &str, connections: usize) {
        let mut accepted: Vec<u64> = self
            .of(Hook::Accept, filter)
            .iter()
            .map(|seen| seen.id)
            .collect();
        let mut closed: Vec<u64> = self
            .of(Hook::Close, filter)
            .iter()
            .map(|seen| seen.id)
            .collect();
        accepted.sort_unstable();
        closed.sort_unstable();
        assert_eq!(accepted.len(), connections, "{:#?}", self.seen());
        assert_eq!(
            accepted,
            closed,
            "every accepted connection is closed exactly once: {:#?}",
            self.seen()
        );
    }
}

type Decide = Box<dyn Fn(u32) -> BoxFuture<'static, FilterVerdict> + Send + Sync>;

struct Watched {
    id: &'static str,
    priority: FilterPriority,
    log: Log,
    calls: AtomicU32,
    decide: Decide,
}

#[derive(Clone)]
struct Probe(Arc<Watched>);

impl Probe {
    fn new(
        id: &'static str,
        log: &Log,
        decide: impl Fn(u32) -> BoxFuture<'static, FilterVerdict> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(Watched {
            id,
            priority: FilterPriority::Normal,
            log: log.clone(),
            calls: AtomicU32::new(0),
            decide: Box::new(decide),
        }))
    }

    fn passing(id: &'static str, log: &Log) -> Self {
        Self::new(id, log, |_| Box::pin(async { FilterVerdict::Continue }))
    }

    fn priority(self, priority: FilterPriority) -> Self {
        let Watched {
            id,
            log,
            calls,
            decide,
            ..
        } = Arc::into_inner(self.0).expect("the probe is not shared yet");
        Self(Arc::new(Watched {
            id,
            priority,
            log,
            calls,
            decide,
        }))
    }
}

impl TransportFilter for Probe {
    fn metadata(&self) -> FilterMetadata {
        let mut metadata = FilterMetadata::new(self.0.id);
        metadata.priority = self.0.priority;
        metadata
    }

    fn on_accept<'a>(&'a self, ctx: &'a mut TransportContext) -> BoxFuture<'a, FilterVerdict> {
        self.0.log.record(Hook::Accept, self.0.id, ctx);
        let call = self.0.calls.fetch_add(1, Ordering::SeqCst);
        (self.0.decide)(call)
    }

    fn on_close(&self, ctx: &TransportContext) {
        self.0.log.record(Hook::Close, self.0.id, ctx);
    }
}

fn gatekeeper(probes: Vec<Probe>) -> ScriptedPlugin {
    ScriptedPlugin::new(OWNER).on_enable(move |ctx| {
        let registry = ctx
            .transport_filters()
            .expect("trusted plugins hold the transport-filter capability");
        for probe in &probes {
            registry
                .register(Box::new(probe.clone()))
                .expect("the filter id is free");
        }
    })
}

fn watching(log: &Log) -> ScriptedPlugin {
    gatekeeper(vec![Probe::passing(WATCH, log)])
}

fn closed_without_answer<R: std::fmt::Debug>(result: Result<R, HarnessError>) {
    match result {
        Err(HarnessError::Closed(_) | HarnessError::Io(_)) => {}
        other => panic!("expected the proxy to close the connection silently, got {other:?}"),
    }
}

fn events(table: &mut Table, key: &str, value: &str) {
    table.insert(
        "events".into(),
        Value::Table(Table::from_iter([(
            key.to_string(),
            Value::String(value.into()),
        )])),
    );
}

async fn offline_proxy(plugin: ScriptedPlugin) -> (TestProxy, FakeBackend) {
    offline_proxy_with(plugin, |builder| builder).await
}

async fn offline_proxy_with(
    plugin: ScriptedPlugin,
    configure: impl FnOnce(TestProxyBuilder) -> TestProxyBuilder,
) -> (TestProxy, FakeBackend) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let builder = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin);
    let proxy = configure(builder).start().await.unwrap();
    (proxy, backend)
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }

    fn warnings(&self) -> Vec<String> {
        self.text()
            .lines()
            .filter(|line| line.contains("WARN"))
            .map(str::to_owned)
            .collect()
    }
}

impl std::io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Logs {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn capture_warnings() -> (Logs, impl Drop) {
    infrarust_test_harness::init_tracing();
    let logs = Logs::default();
    let guard = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(tracing_subscriber::filter::LevelFilter::WARN)
        .finish()
        .set_default();
    (logs, guard)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_filter_only_delays_its_own_connection() {
    let log = Log::new();
    let held = Arc::new(Mutex::new(None::<oneshot::Sender<()>>));
    let release = Arc::new(Notify::new());
    let (held_tx, held_rx) = oneshot::channel();
    *held.lock().unwrap() = Some(held_tx);
    let slow = {
        let held = Arc::clone(&held);
        let release = Arc::clone(&release);
        Probe::new("slow", &log, move |call| {
            let held = held.lock().unwrap().take();
            let release = Arc::clone(&release);
            Box::pin(async move {
                if call == 0 {
                    let wait = release.notified();
                    if let Some(held) = held {
                        let _ = held.send(());
                    }
                    wait.await;
                }
                FilterVerdict::Continue
            })
        })
    };
    let (proxy, _backend) = offline_proxy(gatekeeper(vec![slow])).await;

    let client = proxy.client(VERSION);
    let first = tokio::spawn(async move { client.status().await });
    tokio::time::timeout(T, held_rx)
        .await
        .expect("the first connection reaches the filter")
        .unwrap();

    proxy
        .client(VERSION)
        .status()
        .await
        .expect("a second connection is served while the first one is held by the filter");
    assert!(
        !first.is_finished(),
        "the first connection is still held by its filter"
    );

    release.notify_one();
    first
        .await
        .unwrap()
        .expect("the held connection is served once the filter answers");

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_close_follows_a_status_ping() {
    let log = Log::new();
    let (proxy, _backend) = offline_proxy(watching(&log)).await;

    proxy.client(VERSION).status().await.unwrap();
    log.closes(WATCH, 1).await;

    proxy.shutdown().await.unwrap();
    log.assert_paired(WATCH, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_close_follows_a_login_and_a_client_quit() {
    let log = Log::new();
    let (proxy, backend) = offline_proxy(watching(&log)).await;

    let session = proxy
        .client(VERSION)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let conn = backend.next_connection(T).await.unwrap();
    assert!(
        log.of(Hook::Close, WATCH).is_empty(),
        "the connection is still open"
    );
    session.quit().await;
    conn.closed(T).await.unwrap();
    log.closes(WATCH, 1).await;

    proxy.shutdown().await.unwrap();
    log.assert_paired(WATCH, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_close_follows_a_session_still_open_at_shutdown() {
    let log = Log::new();
    let (proxy, backend) = offline_proxy(watching(&log)).await;

    let _session = proxy
        .client(VERSION)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();
    assert!(log.of(Hook::Close, WATCH).is_empty());

    proxy.shutdown().await.unwrap();
    log.assert_paired(WATCH, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_close_follows_a_connection_the_pipeline_rejects() {
    let log = Log::new();
    let (proxy, backend) = offline_proxy(watching(&log)).await;

    let _ = proxy
        .client(VERSION)
        .domain("nowhere.test")
        .login("Steve")
        .await;
    log.closes(WATCH, 1).await;
    assert_eq!(backend.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
    log.assert_paired(WATCH, 1);
}

async fn on_close_follows_a_forwarded_session(mode: ProxyMode) {
    let log = Log::new();
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::new("lobby", mode).backend(backend.addr()))
        .plugin(watching(&log))
        .start()
        .await
        .unwrap();

    let session = proxy
        .client(VERSION)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    assert!(log.of(Hook::Close, WATCH).is_empty(), "{mode:?}");
    session.quit().await;
    conn.closed(T).await.unwrap();
    conn.close().await;
    log.closes(WATCH, 1).await;

    proxy.shutdown().await.unwrap();
    log.assert_paired(WATCH, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_close_follows_a_passthrough_session() {
    on_close_follows_a_forwarded_session(ProxyMode::Passthrough).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_close_follows_a_zero_copy_session() {
    on_close_follows_a_forwarded_session(ProxyMode::ZeroCopy).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_close_follows_a_server_only_session() {
    on_close_follows_a_forwarded_session(ProxyMode::ServerOnly).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_close_follows_a_legacy_ping() {
    let log = Log::new();
    let ping = LegacyPing {
        protocol: Some(i32::from(LEGACY_PROTOCOL)),
        version: Some("1.6.4".into()),
        motd: "A legacy backend".into(),
        online: 3,
        max: 20,
    };
    let backend = FakeLegacyBackend::spawn(ping.clone()).await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("old").backend(backend.addr()))
        .plugin(watching(&log))
        .start()
        .await
        .unwrap();

    let answered = proxy
        .legacy_client_for("old")
        .unwrap()
        .ping()
        .await
        .unwrap();
    assert_eq!(answered, ping);
    log.closes(WATCH, 1).await;

    proxy.shutdown().await.unwrap();
    log.assert_paired(WATCH, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejection_closes_only_the_filters_that_accepted_before_it() {
    let log = Log::new();
    let recorder = Recorder::new();
    let outer = Probe::passing("outer", &log).priority(FilterPriority::First);
    let inner = Probe::new("inner", &log, |_| Box::pin(async { FilterVerdict::Reject }))
        .priority(FilterPriority::Late);
    let (proxy, _backend) = offline_proxy_with(gatekeeper(vec![inner, outer]), |builder| {
        builder.plugin(recorder.plugin())
    })
    .await;

    closed_without_answer(proxy.client(VERSION).status().await);
    log.closes("outer", 1).await;
    let rejected = recorder
        .wait_for(|e| e.kind == EventKind::ConnectionRejected, T)
        .await
        .unwrap();
    assert_eq!(rejected.detail["reason"], json!("plugin"));
    assert_eq!(rejected.detail["plugin"], json!(OWNER));
    assert_eq!(rejected.detail["virtual_host"], json!(null));

    proxy.shutdown().await.unwrap();
    log.assert_paired("outer", 1);
    assert_eq!(log.of(Hook::Accept, "inner").len(), 1);
    assert!(
        log.of(Hook::Close, "inner").is_empty(),
        "the filter that rejected gets no on_close: {:#?}",
        log.seen()
    );
    let order: Vec<&str> = log
        .seen()
        .iter()
        .filter(|seen| seen.hook == Hook::Accept)
        .map(|seen| seen.filter)
        .collect();
    assert_eq!(order, ["outer", "inner"]);
}

async fn a_faulty_filter_rejects_its_connection_only(
    decide: impl Fn(u32) -> BoxFuture<'static, FilterVerdict> + Send + Sync + 'static,
    patch: impl FnOnce(&mut Table) + Send + 'static,
    fault: &str,
) {
    let (logs, _capture) = capture_warnings();
    let log = Log::new();
    let recorder = Recorder::new();
    let gate = gatekeeper(vec![
        Probe::passing(WATCH, &log).priority(FilterPriority::First),
        Probe::new("faulty", &log, decide),
    ]);
    let (proxy, _backend) = offline_proxy_with(gate, |builder| {
        builder.plugin(recorder.plugin()).patch_config(patch)
    })
    .await;

    closed_without_answer(proxy.client(VERSION).status().await);
    log.closes(WATCH, 1).await;
    let warnings = logs.warnings();
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("faulty") && line.contains(OWNER) && line.contains(fault)),
        "a warning names the filter, its plugin and the fault: {}",
        logs.text()
    );
    let rejected = recorder
        .wait_for(|e| e.kind == EventKind::ConnectionRejected, T)
        .await
        .unwrap();
    assert_eq!(rejected.detail["reason"], json!("plugin"));
    assert_eq!(rejected.detail["plugin"], json!(OWNER));

    proxy
        .client(VERSION)
        .status()
        .await
        .expect("the next connection is served normally");
    log.closes(WATCH, 2).await;

    proxy.shutdown().await.unwrap();
    log.assert_paired(WATCH, 2);
    assert_eq!(log.of(Hook::Close, "faulty").len(), 1);
}

#[tokio::test]
async fn a_panicking_filter_rejects_its_connection_and_the_next_one_is_served() {
    a_faulty_filter_rejects_its_connection_only(
        |call| {
            Box::pin(async move {
                assert!(call > 0, "the filter blows up on its first connection");
                FilterVerdict::Continue
            })
        },
        |_| {},
        "panicked",
    )
    .await;
}

#[tokio::test]
async fn a_stalled_filter_times_out_rejects_its_connection_and_the_next_one_is_served() {
    a_faulty_filter_rejects_its_connection_only(
        |call| {
            Box::pin(async move {
                if call == 0 {
                    std::future::pending::<()>().await;
                }
                FilterVerdict::Continue
            })
        },
        |table| events(table, "transport_filter_timeout", "200ms"),
        "timed out",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_ip_is_the_address_from_the_proxy_protocol_header() {
    let log = Log::new();
    let (proxy, _backend) = offline_proxy_with(watching(&log), |builder| {
        builder.patch_config(|table| {
            table.insert("receive_proxy_protocol".into(), Value::Boolean(true));
        })
    })
    .await;
    let source: SocketAddr = "203.0.113.7:50001".parse().unwrap();

    FakeClient::new(proxy.addr(), VERSION)
        .domain("lobby.test")
        .proxy_protocol(source)
        .status()
        .await
        .unwrap();

    let accepted = log.of(Hook::Accept, WATCH);
    assert_eq!(accepted.len(), 1, "{accepted:#?}");
    assert_eq!(accepted[0].real_ip, Some(source.ip()));
    assert!(accepted[0].remote.ip().is_loopback(), "{accepted:#?}");

    proxy.shutdown().await.unwrap();
}

struct SessionIds(Arc<Mutex<Vec<u64>>>);

impl CodecFilterFactory for SessionIds {
    fn metadata(&self) -> FilterMetadata {
        FilterMetadata::new("session_ids")
    }

    fn create(&self, init: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
        self.0.lock().unwrap().push(init.connection_id);
        Box::new(Untouched)
    }
}

struct Untouched;

impl CodecFilterInstance for Untouched {
    fn filter(&mut self, _packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
        CodecVerdict::Pass
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_connection_id_is_the_player_id_of_the_session() {
    let log = Log::new();
    let players = Arc::new(Mutex::new(Vec::new()));
    let codecs = Arc::new(Mutex::new(Vec::new()));
    let seen_players = Arc::clone(&players);
    let seen_codecs = Arc::clone(&codecs);
    let plugin = watching(&log)
        .on::<PostLoginEvent>(EventPriority::NORMAL, move |event| {
            seen_players
                .lock()
                .unwrap()
                .push(event.player.id().as_u64());
        })
        .on_enable(move |ctx| {
            ctx.codec_filters()
                .expect("trusted plugins hold the codec-filter capability")
                .register(Box::new(SessionIds(Arc::clone(&seen_codecs))))
                .expect("the filter id is free");
        });
    let (proxy, backend) = offline_proxy(plugin).await;

    proxy.client(VERSION).status().await.unwrap();
    let session = proxy
        .client(VERSION)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();
    session.quit().await;
    log.closes(WATCH, 2).await;

    let accepted: Vec<u64> = log
        .of(Hook::Accept, WATCH)
        .iter()
        .map(|seen| seen.id)
        .collect();
    assert_eq!(accepted.len(), 2);
    assert!(accepted.iter().all(|id| *id != 0), "{accepted:?}");
    assert_ne!(accepted[0], accepted[1], "each connection has its own id");
    assert_eq!(*players.lock().unwrap(), [accepted[1]]);
    assert_eq!(
        *codecs.lock().unwrap(),
        [accepted[1], accepted[1]],
        "both codec chains of the session carry the same id"
    );

    proxy.shutdown().await.unwrap();
}
