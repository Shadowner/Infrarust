#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

pub(crate) const TEST_TARGET: &str = "x86_64-unknown-linux-musl";

pub(crate) struct SigningKey {
    public: minisign::PublicKey,
    secret: minisign::SecretKey,
}

impl SigningKey {
    pub(crate) fn generate() -> Self {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        Self {
            public: pair.pk,
            secret: pair.sk,
        }
    }

    pub(crate) fn public(&self) -> String {
        self.public.to_box().unwrap().into_string()
    }

    pub(crate) fn sign(&self, data: &[u8]) -> String {
        minisign::sign(
            Some(&self.public),
            &self.secret,
            Cursor::new(data),
            Some("file:manifest.json"),
            None,
        )
        .unwrap()
        .into_string()
    }
}

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn manifest_json(version: &str, artifacts: &[(&str, &str, &[u8])]) -> String {
    let artifacts: Vec<serde_json::Value> = artifacts
        .iter()
        .map(|(file, binary, archive)| {
            serde_json::json!({
                "targets": [TEST_TARGET],
                "file": file,
                "binary": binary,
                "sha256": sha256_hex(archive),
                "size": archive.len(),
            })
        })
        .collect();
    serde_json::json!({
        "schema": 1,
        "version": version,
        "tag": format!("v{version}"),
        "published_at": "2026-11-24T10:00:00Z",
        "notes_url": format!("https://example.invalid/notes/{version}"),
        "security": false,
        "handover": null,
        "artifacts": artifacts,
    })
    .to_string()
}

#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub(crate) path: String,
    pub(crate) query: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
}

impl Request {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    stall: bool,
}

impl Reply {
    pub(crate) fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
            stall: false,
        }
    }

    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            ..Self::ok(Vec::new())
        }
    }

    pub(crate) fn stall() -> Self {
        Self {
            stall: true,
            ..Self::ok(Vec::new())
        }
    }

    pub(crate) fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

type Handler = dyn Fn(&Request) -> Reply + Send + Sync;
type Routes = Arc<Mutex<HashMap<String, Reply>>>;

pub(crate) const ARCHIVE: &str = "infrarust-linux-x86_64.tar.gz";

pub(crate) struct HttpServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<Request>>>,
    routes: Option<Routes>,
    task: tokio::task::JoinHandle<()>,
}

impl HttpServer {
    pub(crate) async fn start(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let log = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let handler = Arc::clone(&handler);
                let log = Arc::clone(&log);
                tokio::spawn(serve(stream, handler, log));
            }
        });
        Self {
            addr,
            requests,
            routes: None,
            task,
        }
    }

    pub(crate) async fn routed() -> Self {
        let routes: Routes = Arc::default();
        let table = Arc::clone(&routes);
        let mut server = Self::start(move |request| {
            table
                .lock()
                .unwrap()
                .get(&request.path)
                .cloned()
                .unwrap_or_else(|| Reply::status(404))
        })
        .await;
        server.routes = Some(routes);
        server
    }

    pub(crate) fn route(&self, path: &str, reply: Reply) {
        self.routes
            .as_ref()
            .expect("a routed server")
            .lock()
            .unwrap()
            .insert(path.to_string(), reply);
    }

    pub(crate) fn publish(&self, tag: &str, assets: &[(&str, &[u8])]) -> serde_json::Value {
        let assets: Vec<serde_json::Value> = assets
            .iter()
            .map(|(name, bytes)| {
                let path = format!("/assets/{tag}/{name}");
                self.route(&path, Reply::ok(bytes.to_vec()));
                serde_json::json!({
                    "name": name,
                    "browser_download_url": self.url(&path),
                    "size": bytes.len(),
                })
            })
            .collect();
        serde_json::json!({
            "tag_name": tag,
            "draft": false,
            "prerelease": false,
            "published_at": "2026-11-24T10:00:00Z",
            "html_url": format!("https://example.invalid/releases/{tag}"),
            "assets": assets,
        })
    }

    pub(crate) fn publish_signed(
        &self,
        key: &SigningKey,
        version: &str,
        archive: &[u8],
    ) -> serde_json::Value {
        let manifest = manifest_json(version, &[(ARCHIVE, "infrarust", archive)]);
        let signature = key.sign(manifest.as_bytes());
        self.publish(
            &format!("v{version}"),
            &[
                ("manifest.json", manifest.as_bytes()),
                ("manifest.json.minisig", signature.as_bytes()),
                (ARCHIVE, archive),
            ],
        )
    }

    pub(crate) fn list(&self, releases: &[serde_json::Value]) {
        self.route(
            "/releases",
            Reply::ok(serde_json::to_vec(releases).unwrap()),
        );
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    pub(crate) fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(
    mut stream: tokio::net::TcpStream,
    handler: Arc<Handler>,
    log: Arc<Mutex<Vec<Request>>>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut head = Vec::new();
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read_buf(&mut head).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n");
    let target = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or("/")
        .to_string();
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), Some(query.to_string())),
        None => (target, None),
    };
    let headers = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect();
    let request = Request {
        path,
        query,
        headers,
    };
    log.lock().unwrap().push(request.clone());
    let reply = handler(&request);
    if reply.stall {
        std::future::pending::<()>().await;
    }
    let mut response = format!("HTTP/1.1 {} Test\r\n", reply.status);
    for (key, value) in &reply.headers {
        response.push_str(&format!("{key}: {value}\r\n"));
    }
    response.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        reply.body.len()
    ));
    let mut bytes = response.into_bytes();
    bytes.extend_from_slice(&reply.body);
    let _ = stream.write_all(&bytes).await;
    let _ = stream.shutdown().await;
}
