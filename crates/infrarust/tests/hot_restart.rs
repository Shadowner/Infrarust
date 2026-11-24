#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind, Write as _};
use std::net::SocketAddr;
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use command_fds::{CommandFdExt, FdMapping};
use infrarust_core::player::RESTART_REASON;
use infrarust_test_harness::{
    BackendConn, ClientSession, FakeBackend, FakeClient, ProtocolVersion,
};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

const BIN: &str = env!("CARGO_BIN_EXE_infrarust");
const T: Duration = Duration::from_secs(30);
const PROTOCOL: u32 = 769;
const OFFLINE: ProtocolVersion = ProtocolVersion(777);
const READY: &str = "infrarust is ready, accepting connections";
const COMMITTED: &str = "handover committed";
const FAILPOINT: &str = "INFRARUST_HANDOVER_FAILPOINT";

fn varint(mut value: u32, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn string(text: &str, out: &mut Vec<u8>) {
    varint(u32::try_from(text.len()).unwrap(), out);
    out.extend_from_slice(text.as_bytes());
}

fn frame(body: &[u8], out: &mut Vec<u8>) {
    varint(u32::try_from(body.len()).unwrap(), out);
    out.extend_from_slice(body);
}

fn hello(domain: &str, name: &str) -> Vec<u8> {
    let mut handshake = Vec::new();
    varint(0, &mut handshake);
    varint(PROTOCOL, &mut handshake);
    string(domain, &mut handshake);
    handshake.extend_from_slice(&25565u16.to_be_bytes());
    varint(2, &mut handshake);
    let mut login = Vec::new();
    varint(0, &mut login);
    string(name, &mut login);
    login.extend_from_slice(&[7; 16]);
    let mut out = Vec::new();
    frame(&handshake, &mut out);
    frame(&login, &mut out);
    out
}

async fn read_varint(stream: &mut TcpStream) -> std::io::Result<u32> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = stream.read_u8().await?;
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(ErrorKind::InvalidData.into())
}

async fn read_frame(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let len = read_varint(stream).await? as usize;
    let mut body = vec![0; len];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

async fn read_hello(stream: &mut TcpStream) -> std::io::Result<String> {
    read_frame(stream).await?;
    let login = read_frame(stream).await?;
    let len = usize::from(*login.get(1).ok_or(ErrorKind::InvalidData)?);
    let name = login.get(2..2 + len).ok_or(ErrorKind::InvalidData)?;
    String::from_utf8(name.to_vec()).map_err(|_| ErrorKind::InvalidData.into())
}

struct RawBackend {
    addr: SocketAddr,
    arrivals: tokio::sync::Mutex<mpsc::UnboundedReceiver<(String, TcpStream)>>,
    waiting: Mutex<HashMap<String, TcpStream>>,
}

impl RawBackend {
    async fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (arrived, arrivals) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let arrived = arrived.clone();
                tokio::spawn(async move {
                    if let Ok(name) = read_hello(&mut stream).await {
                        let _ = arrived.send((name, stream));
                    }
                });
            }
        });
        Self {
            addr,
            arrivals: tokio::sync::Mutex::new(arrivals),
            waiting: Mutex::new(HashMap::new()),
        }
    }

    async fn connection(&self, name: &str) -> TcpStream {
        let found = tokio::time::timeout(T, async {
            loop {
                if let Some(stream) = self
                    .waiting
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(name)
                {
                    return stream;
                }
                let (arrived, stream) = self.arrivals.lock().await.recv().await.unwrap();
                if arrived == name {
                    return stream;
                }
                self.waiting
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(arrived, stream);
            }
        })
        .await;
        found.unwrap_or_else(|_| panic!("the backend never saw {name}"))
    }
}

fn numbers(from: u64, count: u64) -> Vec<u8> {
    (from..from + count).flat_map(u64::to_be_bytes).collect()
}

async fn expect_numbers(stream: &mut TcpStream, from: u64, count: u64, what: &str) {
    let mut bytes = vec![0; usize::try_from(count * 8).unwrap()];
    match tokio::time::timeout(T, stream.read_exact(&mut bytes)).await {
        Err(_) => panic!("{what}: numbers {from}.. never arrived"),
        Ok(Err(error)) => panic!("{what}: the stream broke before {from}..: {error}"),
        Ok(Ok(_)) => {}
    }
    for (offset, chunk) in (0..).zip(bytes.as_chunks::<8>().0) {
        let number = u64::from_be_bytes(*chunk);
        assert_eq!(number, from + offset, "{what}: numbering broke");
    }
}

struct Raw {
    name: String,
    client: TcpStream,
    backend: TcpStream,
    sent: u64,
    received: u64,
}

impl Raw {
    async fn connect(addr: SocketAddr, server: &str, name: &str) -> TcpStream {
        let mut client = tokio::time::timeout(T, TcpStream::connect(addr))
            .await
            .expect("the proxy never accepted the connection")
            .unwrap();
        client
            .write_all(&hello(&format!("{server}.test"), name))
            .await
            .unwrap();
        client
    }

    async fn attach(client: TcpStream, backend: &RawBackend, name: &str) -> Self {
        Self {
            name: name.to_string(),
            client,
            backend: backend.connection(name).await,
            sent: 0,
            received: 0,
        }
    }

    async fn join(addr: SocketAddr, server: &str, backend: &RawBackend, name: &str) -> Self {
        let client = Self::connect(addr, server, name).await;
        Self::attach(client, backend, name).await
    }

    async fn send(&mut self, count: u64) {
        let bytes = numbers(self.sent, count);
        self.client.write_all(&bytes).await.unwrap();
        self.backend.write_all(&bytes).await.unwrap();
        self.sent += count;
    }

    async fn receive(&mut self) {
        let count = self.sent - self.received;
        let name = &self.name;
        expect_numbers(
            &mut self.backend,
            self.received,
            count,
            &format!("{name} up"),
        )
        .await;
        expect_numbers(
            &mut self.client,
            self.received,
            count,
            &format!("{name} down"),
        )
        .await;
        self.received = self.sent;
    }

    async fn exchange(&mut self, count: u64) {
        self.send(count).await;
        self.receive().await;
    }

    fn assert_open(&self) {
        for (side, stream) in [("client", &self.client), ("backend", &self.backend)] {
            let mut byte = [0; 1];
            match stream.try_read(&mut byte) {
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                other => panic!(
                    "{} {side}: expected an idle open stream, got {other:?}",
                    self.name
                ),
            }
        }
    }
}

async fn join_all(
    addr: SocketAddr,
    backend: &RawBackend,
    pipes: usize,
    splices: usize,
) -> Vec<Raw> {
    let mut sessions = Vec::with_capacity(pipes + splices);
    for index in 0..pipes {
        sessions.push(Raw::join(addr, "pipe", backend, &format!("Pipe{index}")).await);
    }
    for index in 0..splices {
        sessions.push(Raw::join(addr, "splice", backend, &format!("Splice{index}")).await);
    }
    sessions
}

async fn send_all(sessions: &mut [Raw], count: u64) {
    for session in sessions {
        session.send(count).await;
    }
}

async fn receive_all(sessions: &mut [Raw]) {
    for session in sessions {
        session.receive().await;
    }
}

async fn exchange_all(sessions: &mut [Raw], count: u64) {
    send_all(sessions, count).await;
    receive_all(sessions).await;
}

struct Site {
    dir: tempfile::TempDir,
    exe: PathBuf,
    config: PathBuf,
    listener: Option<std::net::TcpListener>,
    addr: SocketAddr,
}

impl Site {
    fn new(servers: &[(&str, &str, SocketAddr)]) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("hot-restart-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let exe = dir.path().join("infrarust");
        if std::fs::hard_link(BIN, &exe).is_err() {
            std::fs::copy(BIN, &exe).unwrap();
        }
        let servers_dir = dir.path().join("servers");
        let plugins_dir = dir.path().join("plugins");
        std::fs::create_dir(&servers_dir).unwrap();
        std::fs::create_dir(&plugins_dir).unwrap();
        for (id, mode, backend) in servers {
            std::fs::write(
                servers_dir.join(format!("{id}.toml")),
                format!(
                    "domains = [\"{id}.test\"]\naddresses = [\"{backend}\"]\nproxy_mode = \"{mode}\"\n"
                ),
            )
            .unwrap();
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let config = dir.path().join("infrarust.toml");
        std::fs::write(
            &config,
            format!(
                "bind = \"{addr}\"\nservers_dir = {:?}\nplugins_dir = {:?}\nmax_connections_per_ip = 0\n\n[rate_limit]\nenabled = false\n\n[update]\ncheck = false\n\n[ban]\nfile = {:?}\n",
                servers_dir.display().to_string(),
                plugins_dir.display().to_string(),
                dir.path().join("bans.json").display().to_string(),
            ),
        )
        .unwrap();
        Self {
            dir,
            exe,
            config,
            listener: Some(listener),
            addr,
        }
    }

    fn replace_binary(&self) -> u64 {
        let staged = self.dir.path().join(".infrarust.next");
        std::fs::copy(BIN, &staged).unwrap();
        std::fs::rename(&staged, &self.exe).unwrap();
        std::fs::metadata(&self.exe).unwrap().ino()
    }
}

struct Proxy {
    child: Child,
    stdin: Option<ChildStdin>,
    logs: Arc<Mutex<Vec<String>>>,
    pid: Pid,
}

impl Proxy {
    fn start(site: &mut Site, env: &[(&str, &str)]) -> Self {
        let mut command = Command::new(&site.exe);
        command
            .arg("-c")
            .arg(&site.config)
            .current_dir(site.dir.path())
            .env("NO_COLOR", "1")
            .env("LISTEN_FDS", "1")
            .env("LISTEN_FDNAMES", "game")
            .env_remove("RUST_LOG")
            .env_remove(FAILPOINT)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            command.env(key, value);
        }
        let listener = site.listener.take().expect("one proxy per site");
        command
            .fd_mappings(vec![FdMapping {
                parent_fd: OwnedFd::from(listener),
                child_fd: 3,
            }])
            .unwrap();
        let mut child = command.spawn().unwrap();
        let logs = Arc::new(Mutex::new(Vec::new()));
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        for output in [
            Box::new(stdout) as Box<dyn std::io::Read + Send>,
            Box::new(stderr),
        ] {
            let logs = Arc::clone(&logs);
            std::thread::spawn(move || {
                for line in BufReader::new(output).lines() {
                    let Ok(line) = line else {
                        return;
                    };
                    logs.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(line);
                }
            });
        }
        let pid = Pid::from_raw(i32::try_from(child.id()).unwrap());
        Self {
            stdin: child.stdin.take(),
            child,
            logs,
            pid,
        }
    }

    fn count(&self, needle: &str) -> usize {
        self.logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|line| line.contains(needle))
            .count()
    }

    fn tail(&self) -> String {
        let logs = self.logs.lock().unwrap_or_else(PoisonError::into_inner);
        logs[logs.len().saturating_sub(60)..].join("\n")
    }

    async fn wait_for(&mut self, needle: &str, times: usize) {
        let deadline = tokio::time::Instant::now() + T;
        while self.count(needle) < times {
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!(
                    "the proxy exited with {status} while waiting for {needle:?} x{times}:\n{}",
                    self.tail()
                );
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {needle:?} x{times}:\n{}",
                self.tail()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn restarted(&mut self, restarts: usize) {
        self.wait_for(COMMITTED, restarts).await;
        self.wait_for(READY, restarts + 1).await;
        assert!(self.child.try_wait().unwrap().is_none());
    }

    fn signal(&self, signal: Signal) {
        kill(self.pid, signal).unwrap();
    }

    fn console(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
    }

    fn environ(&self) -> Vec<String> {
        std::fs::read(format!("/proc/{}/environ", self.pid))
            .unwrap()
            .split(|byte| *byte == 0)
            .map(|entry| String::from_utf8_lossy(entry).into_owned())
            .collect()
    }

    fn assert_image(&self, attempt: &str) {
        let environ = self.environ();
        assert!(
            environ.contains(&format!("INFRARUST_HANDOVER_ATTEMPT={attempt}")),
            "{environ:?}"
        );
        assert!(
            !environ.iter().any(|entry| entry.starts_with("LISTEN_")),
            "socket activation leaked into the next image: {environ:?}"
        );
    }

    fn exe_inode(&self) -> u64 {
        std::fs::metadata(format!("/proc/{}/exe", self.pid))
            .unwrap()
            .ino()
    }

    async fn stop(mut self) {
        self.signal(Signal::SIGTERM);
        let deadline = tokio::time::Instant::now() + T;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "{status}:\n{}", self.tail());
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the proxy did not stop:\n{}",
                self.tail()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

async fn offline_player(
    addr: SocketAddr,
    lobby: &FakeBackend,
    name: &str,
) -> (ClientSession, BackendConn) {
    let client = FakeClient::new(addr, OFFLINE)
        .domain("lobby.test")
        .login(name)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let backend = lobby.next_connection(T).await.unwrap();
    (client, backend)
}

fn forwarding_site(backend: &RawBackend) -> Site {
    Site::new(&[
        ("pipe", "passthrough", backend.addr),
        ("splice", "zero_copy", backend.addr),
    ])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forwarding_sessions_survive_hot_restarts_in_the_same_process() {
    let backend = RawBackend::spawn().await;
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let mut site = Site::new(&[
        ("pipe", "passthrough", backend.addr),
        ("splice", "zero_copy", backend.addr),
        ("lobby", "offline", lobby.addr()),
    ]);
    let addr = site.addr;
    let mut proxy = Proxy::start(&mut site, &[]);
    proxy.wait_for(READY, 1).await;
    let pid = proxy.pid;

    let mut sessions = join_all(addr, &backend, 8, 2).await;
    exchange_all(&mut sessions, 50).await;
    let (mut notch, _notch_backend) = offline_player(addr, &lobby, "Notch").await;
    send_all(&mut sessions, 50).await;

    proxy.signal(Signal::SIGUSR2);
    let during = tokio::spawn(Raw::connect(addr, "pipe", "During"));
    let kicked = notch.expect_disconnect(T).await.unwrap();
    assert_eq!(kicked.text, RESTART_REASON, "{kicked:?}");
    proxy.restarted(1).await;

    assert_eq!(proxy.pid, pid);
    proxy.assert_image("1");
    receive_all(&mut sessions).await;
    exchange_all(&mut sessions, 50).await;
    let mut during = Raw::attach(during.await.unwrap(), &backend, "During").await;
    during.exchange(20).await;
    let mut after = Raw::join(addr, "splice", &backend, "After").await;
    after.exchange(20).await;
    sessions.push(during);
    sessions.push(after);
    let (mut back, _back_backend) = offline_player(addr, &lobby, "Notch").await;

    send_all(&mut sessions, 30).await;
    proxy.console("restart");
    let kicked = back.expect_disconnect(T).await.unwrap();
    assert_eq!(kicked.text, RESTART_REASON, "{kicked:?}");
    proxy.restarted(2).await;

    assert_eq!(proxy.pid, pid);
    receive_all(&mut sessions).await;
    exchange_all(&mut sessions, 30).await;
    for session in &sessions {
        session.assert_open();
    }
    proxy.stop().await;
}

async fn a_failed_successor_keeps_the_sessions(failpoint: &str, trace: &str) {
    let backend = RawBackend::spawn().await;
    let mut site = forwarding_site(&backend);
    let addr = site.addr;
    let mut proxy = Proxy::start(&mut site, &[(FAILPOINT, failpoint)]);
    proxy.wait_for(READY, 1).await;
    let pid = proxy.pid;
    let mut sessions = join_all(addr, &backend, 2, 1).await;
    exchange_all(&mut sessions, 40).await;
    send_all(&mut sessions, 40).await;

    proxy.signal(Signal::SIGUSR2);
    proxy.wait_for(trace, 1).await;
    proxy.restarted(1).await;

    assert_eq!(proxy.pid, pid);
    proxy.assert_image("2");
    receive_all(&mut sessions).await;
    exchange_all(&mut sessions, 40).await;
    let mut fresh = Raw::join(addr, "pipe", &backend, "Fresh").await;
    fresh.exchange(10).await;
    sessions.push(fresh);

    send_all(&mut sessions, 10).await;
    proxy.signal(Signal::SIGUSR2);
    proxy.restarted(2).await;
    proxy.assert_image("1");
    assert_eq!(proxy.pid, pid);
    receive_all(&mut sessions).await;
    exchange_all(&mut sessions, 10).await;
    for session in &sessions {
        session.assert_open();
    }
    proxy.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_successor_that_fails_before_its_commit_rolls_back_to_the_running_image() {
    a_failed_successor_keeps_the_sessions(
        "successor-precommit",
        "infrarust: rolling the hot restart back",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_target_that_cannot_start_hands_the_sessions_back_to_the_running_image() {
    a_failed_successor_keeps_the_sessions(
        "exec-target",
        "infrarust: the exec-target failpoint keeps",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "heavy: 200 sessions"]
async fn two_hundred_sessions_survive_a_hot_restart() {
    let backend = RawBackend::spawn().await;
    let mut site = forwarding_site(&backend);
    let addr = site.addr;
    let mut proxy = Proxy::start(&mut site, &[]);
    proxy.wait_for(READY, 1).await;
    let mut sessions = join_all(addr, &backend, 160, 40).await;
    exchange_all(&mut sessions, 20).await;
    send_all(&mut sessions, 20).await;

    proxy.signal(Signal::SIGUSR2);
    proxy.restarted(1).await;

    assert_eq!(proxy.count("restored=200 dropped=0"), 1, "{}", proxy.tail());
    receive_all(&mut sessions).await;
    exchange_all(&mut sessions, 20).await;
    proxy.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "heavy: 10 restarts in a row"]
async fn ten_hot_restarts_in_a_row_keep_every_session() {
    let backend = RawBackend::spawn().await;
    let mut site = forwarding_site(&backend);
    let addr = site.addr;
    let mut proxy = Proxy::start(&mut site, &[]);
    proxy.wait_for(READY, 1).await;
    let pid = proxy.pid;
    let mut sessions = join_all(addr, &backend, 3, 2).await;
    for restart in 1..=10 {
        send_all(&mut sessions, 25).await;
        if restart % 2 == 0 {
            proxy.console("restart");
        } else {
            proxy.signal(Signal::SIGUSR2);
        }
        proxy.restarted(restart).await;
        assert_eq!(proxy.pid, pid);
        receive_all(&mut sessions).await;
        exchange_all(&mut sessions, 25).await;
    }
    for session in &sessions {
        session.assert_open();
    }
    proxy.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "heavy: splice under backpressure"]
async fn spliced_bytes_in_flight_survive_a_hot_restart_under_backpressure() {
    const COUNT: u64 = 1 << 22;
    let backend = RawBackend::spawn().await;
    let mut site = forwarding_site(&backend);
    let addr = site.addr;
    let mut proxy = Proxy::start(&mut site, &[]);
    proxy.wait_for(READY, 1).await;
    let Raw {
        client,
        backend: server,
        ..
    } = Raw::join(addr, "splice", &backend, "Stuck").await;
    let (client_read, mut client_write) = client.into_split();
    let (server_read, mut server_write) = server.into_split();
    let up = tokio::spawn(async move { client_write.write_all(&numbers(0, COUNT)).await });
    let down = tokio::spawn(async move { server_write.write_all(&numbers(0, COUNT)).await });

    proxy.signal(Signal::SIGUSR2);
    proxy.restarted(1).await;

    let read_all = |mut reader: tokio::net::tcp::OwnedReadHalf, what: &'static str| async move {
        let mut bytes = vec![0; usize::try_from(COUNT * 8).unwrap()];
        tokio::time::timeout(T, reader.read_exact(&mut bytes))
            .await
            .unwrap_or_else(|_| panic!("{what}: the bytes never arrived"))
            .unwrap_or_else(|error| panic!("{what}: {error}"));
        for (expected, chunk) in (0..).zip(bytes.as_chunks::<8>().0) {
            assert_eq!(u64::from_be_bytes(*chunk), expected, "{what}");
        }
    };
    tokio::join!(read_all(server_read, "up"), read_all(client_read, "down"));
    up.await.unwrap().unwrap();
    down.await.unwrap().unwrap();
    proxy.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "heavy: update by rename"]
async fn a_binary_replaced_on_disk_takes_the_sessions_over() {
    let backend = RawBackend::spawn().await;
    let mut site = forwarding_site(&backend);
    let addr = site.addr;
    let mut proxy = Proxy::start(&mut site, &[]);
    proxy.wait_for(READY, 1).await;
    let pid = proxy.pid;
    let before = proxy.exe_inode();
    let mut sessions = join_all(addr, &backend, 3, 1).await;
    exchange_all(&mut sessions, 30).await;

    let replaced = site.replace_binary();
    assert_ne!(replaced, before);
    send_all(&mut sessions, 30).await;
    proxy.signal(Signal::SIGUSR2);
    proxy.restarted(1).await;

    assert_eq!(proxy.pid, pid);
    assert_eq!(proxy.exe_inode(), replaced);
    receive_all(&mut sessions).await;
    exchange_all(&mut sessions, 30).await;
    proxy.stop().await;
}
