use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use infrarust_core::auth::game_profile::offline_uuid;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::Instant;

use crate::error::{HarnessError, HarnessResult};

const HAS_JOINED_PATH: &str = "/session/minecraft/hasJoined";
const MAX_REQUEST_HEAD: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCall {
    pub username: String,
    pub server_id: String,
}

#[derive(Debug, Default)]
struct SessionState {
    rejected: Mutex<HashSet<String>>,
    calls: Mutex<Vec<SessionCall>>,
    changed: Notify,
}

impl SessionState {
    fn is_rejected(&self, username: &str) -> bool {
        self.rejected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(username)
    }

    fn record(&self, call: SessionCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        self.changed.notify_waiters();
    }

    fn find(&self, username: &str) -> Option<SessionCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|call| call.username == username)
            .cloned()
    }
}

pub struct FakeSessionServer {
    addr: SocketAddr,
    state: Arc<SessionState>,
    accept_task: JoinHandle<()>,
}

impl std::fmt::Debug for FakeSessionServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeSessionServer")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl FakeSessionServer {
    pub async fn spawn() -> HarnessResult<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(SessionState::default());
        let accept_task = tokio::spawn(accept_loop(listener, Arc::clone(&state)));
        Ok(Self {
            addr,
            state,
            accept_task,
        })
    }

    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn url(&self) -> String {
        format!("http://{}{HAS_JOINED_PATH}", self.addr)
    }

    pub fn accept_all(&self) {
        self.state
            .rejected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    pub fn reject(&self, username: impl Into<String>) {
        self.state
            .rejected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(username.into());
    }

    pub fn calls(&self) -> Vec<(String, String)> {
        self.state
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|call| (call.username.clone(), call.server_id.clone()))
            .collect()
    }

    pub async fn wait_for_call(
        &self,
        username: &str,
        timeout: Duration,
    ) -> HarnessResult<SessionCall> {
        let deadline = Instant::now() + timeout;
        loop {
            let changed = self.state.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(call) = self.state.find(username) {
                return Ok(call);
            }
            tokio::time::timeout_at(deadline, changed)
                .await
                .map_err(|_| {
                    HarnessError::timeout(format!("a hasJoined call for {username}"), timeout)
                })?;
        }
    }
}

impl Drop for FakeSessionServer {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

async fn accept_loop(listener: TcpListener, state: Arc<SessionState>) {
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let state = Arc::clone(&state);
                tasks.spawn(async move {
                    if let Err(e) = serve(stream, &state).await {
                        eprintln!("fake session server connection failed: {e}");
                    }
                });
            }
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
        }
    }
}

async fn serve(mut stream: TcpStream, state: &SessionState) -> HarnessResult<()> {
    let Some(target) = read_request_target(&mut stream).await? else {
        return Ok(());
    };
    let (status, body) = respond(&target, state);
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn read_request_target(stream: &mut TcpStream) -> HarnessResult<Option<String>> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        if head.len() > MAX_REQUEST_HEAD {
            return Err(HarnessError::Unexpected(
                "HTTP request head too large".to_string(),
            ));
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        head.extend_from_slice(&chunk[..read]);
    }
    let head = String::from_utf8_lossy(&head);
    let request_line = head.lines().next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) => Ok(Some(target.to_string())),
        _ => Ok(Some(String::new())),
    }
}

fn respond(target: &str, state: &SessionState) -> (&'static str, String) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != HAS_JOINED_PATH {
        return ("404 Not Found", String::new());
    }
    let mut username = None;
    let mut server_id = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "username" => username = Some(percent_decode(value)),
            "serverId" => server_id = Some(percent_decode(value)),
            _ => {}
        }
    }
    let (Some(username), Some(server_id)) = (username, server_id) else {
        return ("400 Bad Request", String::new());
    };
    let rejected = state.is_rejected(&username);
    state.record(SessionCall {
        username: username.clone(),
        server_id,
    });
    if rejected {
        return ("204 No Content", String::new());
    }
    let profile = json!({
        "id": offline_uuid(&username).simple().to_string(),
        "name": username,
        "properties": [],
    });
    ("200 OK", profile.to_string())
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let decoded = match bytes[i] {
            b'+' => Some((b' ', 1)),
            b'%' => bytes
                .get(i + 1..i + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .map(|byte| (byte, 3)),
            _ => None,
        };
        let (byte, width) = decoded.unwrap_or((bytes[i], 1));
        out.push(byte);
        i += width;
    }
    String::from_utf8_lossy(&out).into_owned()
}
