#![allow(dead_code)]

use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_api::plugin::Plugin;
use infrarust_core::services::command_manager::DispatchOutcome;
use infrarust_loader_wasm::WasmPluginLoader;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::Mutex;

use crate::support::{EnvOptions, TestEnv, loader_from_toml, make_env_with, stage};

pub const PROBE: &str = "net-probe";
pub const PROMPTLY: Duration = Duration::from_secs(20);
pub const QUIET: Duration = Duration::from_millis(300);

pub struct Probe {
    pub tmp: tempfile::TempDir,
    pub plugins_dir: PathBuf,
    pub data: PathBuf,
    pub env: TestEnv,
    pub loader: WasmPluginLoader,
    pub plugin: Box<dyn Plugin>,
    next: AtomicUsize,
}

pub async fn enable_probe(grants: &[&str], proxy_toml: &str) -> Probe {
    try_enable_probe(grants, proxy_toml)
        .await
        .unwrap_or_else(|error| panic!("load {PROBE}: {error}"))
}

pub async fn try_enable_probe(grants: &[&str], proxy_toml: &str) -> Result<Probe, String> {
    let (tmp, plugins_dir) = stage(PROBE);
    let options = grants.iter().fold(EnvOptions::default(), |options, grant| {
        options.grant(PROBE, grant)
    });
    let env = make_env_with(plugins_dir.clone(), options);
    let loader = loader_from_toml(proxy_toml);
    loader.discover(&plugins_dir).await.unwrap();
    let plugin = loader
        .load(PROBE, &env.factory)
        .await
        .map_err(|error| error.to_string())?;
    let ctx = env.factory.create_context(PROBE);
    plugin
        .on_enable(ctx.as_ref())
        .await
        .unwrap_or_else(|error| panic!("enable {PROBE}: {error}"));
    Ok(Probe {
        data: plugins_dir.join(PROBE),
        tmp,
        plugins_dir,
        env,
        loader,
        plugin,
        next: AtomicUsize::new(1),
    })
}

impl Probe {
    pub async fn run(&self, line: &str) -> String {
        let tag = format!("t{}", self.next.fetch_add(1, Ordering::Relaxed));
        let outcome = tokio::time::timeout(
            PROMPTLY,
            self.env
                .command_manager
                .dispatch(crate::support::console(), &format!("probe {tag} {line}")),
        )
        .await
        .expect("a probe answers promptly");
        assert_eq!(outcome, DispatchOutcome::Executed, "probe {line}");
        let prefix = format!("{tag} ");
        std::fs::read_to_string(self.data.join("probe.log"))
            .unwrap_or_default()
            .lines()
            .find_map(|entry| entry.strip_prefix(&prefix).map(str::to_owned))
            .unwrap_or_else(|| format!("no outcome for `{line}`"))
    }

    pub async fn trap(&self) {
        let _ = tokio::time::timeout(
            PROMPTLY,
            self.env
                .command_manager
                .dispatch(crate::support::console(), "probe trap trap"),
        )
        .await
        .expect("a trapping probe returns promptly");
    }
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub fn network_toml(allow: &[String], extra: &str) -> String {
    let rules: Vec<String> = allow.iter().map(|rule| format!("\"{rule}\"")).collect();
    format!(
        "[plugins.{PROBE}.wasm.network]\nallow = [{}]\n{extra}\n",
        rules.join(", ")
    )
}

pub struct EchoServer {
    pub addr: SocketAddr,
    accepts: Arc<AtomicUsize>,
}

impl EchoServer {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&accepts);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 1024];
                    let read = stream.read(&mut buf).await.unwrap_or(0);
                    let _ = stream.write_all(&buf[..read]).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        Self { addr, accepts }
    }

    pub async fn accepts_after_quiet(&self) -> usize {
        tokio::time::sleep(QUIET).await;
        self.accepts.load(Ordering::SeqCst)
    }
}

pub struct SilentListener {
    pub addr: SocketAddr,
    listener: TcpListener,
}

impl SilentListener {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        Self {
            addr: listener.local_addr().unwrap(),
            listener,
        }
    }

    pub async fn saw_no_connection(&self) -> bool {
        tokio::time::timeout(QUIET, self.listener.accept())
            .await
            .is_err()
    }
}

pub struct UdpSink {
    pub addr: SocketAddr,
    socket: UdpSocket,
}

impl UdpSink {
    pub async fn start() -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        Self {
            addr: socket.local_addr().unwrap(),
            socket,
        }
    }

    pub async fn received(&self) -> Option<String> {
        let mut buf = [0u8; 1024];
        match tokio::time::timeout(QUIET * 5, self.socket.recv_from(&mut buf)).await {
            Ok(Ok((read, _))) => Some(String::from_utf8_lossy(&buf[..read]).into_owned()),
            _ => None,
        }
    }
}

pub struct MuteServer {
    pub addr: SocketAddr,
}

impl MuteServer {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let _held = stream;
                    tokio::time::sleep(Duration::from_secs(120)).await;
                });
            }
        });
        Self { addr }
    }
}

pub type Acceptor = Arc<dyn Fn(tokio::net::TcpStream) -> BoxedStream + Send + Sync>;

pub type BoxedStream = Pin<Box<dyn Future<Output = Option<Box<dyn Duplex>>> + Send>>;

pub trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

pub struct HttpServer {
    pub addr: SocketAddr,
    accepts: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl HttpServer {
    pub async fn start(body: &'static str) -> Self {
        Self::start_with(
            body,
            Arc::new(|stream| Box::pin(async move { Some(Box::new(stream) as Box<dyn Duplex>) })),
        )
        .await
    }

    pub async fn start_with(body: &'static str, acceptor: Acceptor) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepts = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (counter, seen) = (Arc::clone(&accepts), Arc::clone(&requests));
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                let (acceptor, seen) = (Arc::clone(&acceptor), Arc::clone(&seen));
                tokio::spawn(async move {
                    let Some(mut stream) = acceptor(stream).await else {
                        return;
                    };
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        match stream.read(&mut byte).await {
                            Ok(1) => head.push(byte[0]),
                            _ => return,
                        }
                    }
                    seen.lock()
                        .await
                        .push(String::from_utf8_lossy(&head).into_owned());
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        Self {
            addr,
            accepts,
            requests,
        }
    }

    pub async fn accepts_after_quiet(&self) -> usize {
        tokio::time::sleep(QUIET).await;
        self.accepts.load(Ordering::SeqCst)
    }

    pub async fn requests(&self) -> Vec<String> {
        self.requests.lock().await.clone()
    }
}
