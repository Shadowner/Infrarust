#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use infrarust_api::event::EventPriority;
use infrarust_api::events::proxy::ConfigReloadEvent;
use infrarust_test_harness::versions::CURRENT;
use infrarust_test_harness::{
    DEFAULT_TIMEOUT, FakeBackend, LoginOutcome, ProtocolVersion, ScriptedPlugin, ServerSpec,
    TestProxy,
};
use toml::{Table, Value};

const T: Duration = DEFAULT_TIMEOUT;
const LOGINS: usize = 200;
const MIN_RELOADS: usize = 5;
const SETTLE: Duration = Duration::from_millis(600);

fn server_file(proxy: &TestProxy, id: &str) -> PathBuf {
    proxy.dir().join("servers").join(format!("{id}.toml"))
}

fn read_table(path: &Path) -> Table {
    toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn with_motd(table: &Table, text: &str) -> String {
    let mut table = table.clone();
    let online = Table::from_iter([("text".to_string(), Value::String(text.to_string()))]);
    table.insert(
        "motd".into(),
        Value::Table(Table::from_iter([(
            "online".to_string(),
            Value::Table(online),
        )])),
    );
    toml::to_string(&table).unwrap()
}

fn reload_counter() -> (ScriptedPlugin, Arc<AtomicUsize>) {
    let reloads = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&reloads);
    let plugin =
        ScriptedPlugin::new("reloads").on::<ConfigReloadEvent>(EventPriority::NORMAL, move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
        });
    (plugin, reloads)
}

fn routed_motd(proxy: &TestProxy, domain: &str) -> Option<String> {
    let (_, config) = proxy.services().domain_router.resolve(domain)?;
    config.motd.online.as_ref().map(|entry| entry.text.clone())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rewriting_a_server_file_keeps_its_domain_routable() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (plugin, reloads) = reload_counter();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .start()
        .await
        .unwrap();
    let domain = proxy.domain("lobby").unwrap().to_string();
    let path = server_file(&proxy, "lobby");
    let original = read_table(&path);

    tokio::time::sleep(SETTLE).await;
    assert_eq!(
        reloads.load(Ordering::SeqCst),
        0,
        "the files the proxy started from were reloaded"
    );

    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let stop = Arc::clone(&stop);
        let path = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut revision = 0usize;
            while !stop.load(Ordering::SeqCst) {
                revision += 1;
                std::fs::write(&path, with_motd(&original, &format!("revision {revision}")))
                    .unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
            revision
        })
    };

    let client = proxy.client_for("lobby", ProtocolVersion(CURRENT)).unwrap();
    let logins = tokio::time::timeout(Duration::from_secs(60), async {
        let mut logins = 0usize;
        while logins < LOGINS || reloads.load(Ordering::SeqCst) < MIN_RELOADS {
            match client.login(&format!("Player{logins}")).await.unwrap() {
                LoginOutcome::Joined(session) => {
                    let _conn = backend.next_connection(T).await.unwrap();
                    session.quit().await;
                }
                LoginOutcome::Disconnected(info) => panic!(
                    "login {logins} was refused after {} reloads: {info:?}",
                    reloads.load(Ordering::SeqCst)
                ),
            }
            logins += 1;
        }
        logins
    })
    .await
    .expect("the logins did not overlap enough reloads");

    stop.store(true, Ordering::SeqCst);
    let revisions = writer.await.unwrap();
    let last = format!("revision {revisions}");
    tokio::time::timeout(T, async {
        while routed_motd(&proxy, &domain).as_deref() != Some(last.as_str()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "the last rewrite never reached the router: {:?}",
            routed_motd(&proxy, &domain)
        )
    });
    assert!(logins >= LOGINS);
    assert!(reloads.load(Ordering::SeqCst) >= MIN_RELOADS);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_backend_keeps_its_port_to_itself() {
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").unreachable())
        .start()
        .await
        .unwrap();
    let table = read_table(&server_file(&proxy, "lobby"));
    let addresses = table["addresses"].as_array().unwrap();
    assert_eq!(addresses.len(), 1);
    let addr: SocketAddr = addresses[0].as_str().unwrap().parse().unwrap();

    let taken = std::net::TcpListener::bind(addr);
    assert_eq!(
        taken.as_ref().map_err(std::io::Error::kind).err(),
        Some(ErrorKind::AddrInUse),
        "another listener could take the unreachable backend {addr}: {taken:?}"
    );

    proxy.shutdown().await.unwrap();
}
