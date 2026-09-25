use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::DEFAULT_TIMEOUT;
use crate::error::{HarnessError, HarnessResult};

pub const LEGACY_PROTOCOL: u8 = 78;

const HANDSHAKE: u8 = 0x02;
const PING: u8 = 0xFE;
const KICK: u8 = 0xFF;
const ENCRYPTION_REQUEST: u8 = 0xFD;
const PING_CHANNEL: &str = "MC|PingHost";
const SECTION: char = '\u{a7}';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyPing {
    pub protocol: Option<i32>,
    pub version: Option<String>,
    pub motd: String,
    pub online: i32,
    pub max: i32,
}

impl LegacyPing {
    fn parse(text: &str) -> HarnessResult<Self> {
        let invalid = || HarnessError::Unexpected(format!("legacy ping reply {text:?}"));
        let number = |value: &str| value.parse::<i32>().map_err(|_| invalid());
        if let Some(fields) = text.strip_prefix("\u{a7}1\0") {
            let parts: Vec<&str> = fields.split('\0').collect();
            let [protocol, version, motd, online, max] = parts[..] else {
                return Err(invalid());
            };
            return Ok(Self {
                protocol: Some(number(protocol)?),
                version: Some(version.to_string()),
                motd: motd.to_string(),
                online: number(online)?,
                max: number(max)?,
            });
        }
        let mut parts = text.rsplitn(3, SECTION);
        let (Some(max), Some(online), Some(motd)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(invalid());
        };
        Ok(Self {
            protocol: None,
            version: None,
            motd: motd.to_string(),
            online: number(online)?,
            max: number(max)?,
        })
    }

    pub fn encode_v1_4(&self) -> Vec<u8> {
        let text = format!(
            "\u{a7}1\0{}\0{}\0{}\0{}\0{}",
            self.protocol.unwrap_or(i32::from(LEGACY_PROTOCOL)),
            self.version.as_deref().unwrap_or("1.6.4"),
            self.motd,
            self.online,
            self.max
        );
        let mut out = vec![KICK];
        out.extend_from_slice(&string16(&text));
        out
    }
}

#[derive(Debug, Clone)]
pub struct LegacyClient {
    addr: SocketAddr,
    hostname: String,
    port: u16,
    protocol: u8,
    timeout: Duration,
}

#[derive(Debug)]
pub enum LegacyLogin {
    Forwarded(LegacySession),
    Kicked(String),
}

impl LegacyLogin {
    pub fn forwarded(self) -> HarnessResult<LegacySession> {
        match self {
            Self::Forwarded(session) => Ok(session),
            Self::Kicked(reason) => Err(HarnessError::Unexpected(format!(
                "expected the legacy login to be forwarded, it was kicked: {reason:?}"
            ))),
        }
    }

    pub fn kicked(self) -> HarnessResult<String> {
        match self {
            Self::Kicked(reason) => Ok(reason),
            Self::Forwarded(session) => Err(HarnessError::Unexpected(format!(
                "expected {} to be kicked, but the login was forwarded",
                session.username
            ))),
        }
    }
}

#[derive(Debug)]
pub struct LegacySession {
    username: String,
    stream: TcpStream,
}

impl LegacySession {
    pub fn username(&self) -> &str {
        &self.username
    }

    pub async fn quit(mut self) {
        let _ = self.stream.shutdown().await;
    }

    pub async fn closed(&mut self, timeout: Duration) -> HarnessResult<()> {
        read_to_end(
            &mut self.stream,
            timeout,
            "the proxy to close the legacy session",
        )
        .await
    }
}

impl LegacyClient {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            hostname: addr.ip().to_string(),
            port: addr.port(),
            protocol: LEGACY_PROTOCOL,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub fn hostname(mut self, hostname: impl Into<String>) -> Self {
        self.hostname = hostname.into();
        self
    }

    #[must_use]
    pub const fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    #[must_use]
    pub const fn protocol(mut self, protocol: u8) -> Self {
        self.protocol = protocol;
        self
    }

    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub async fn login(&self, username: &str) -> HarnessResult<LegacyLogin> {
        tokio::time::timeout(self.timeout, self.login_exchange(username))
            .await
            .map_err(|_| HarnessError::timeout("a legacy login", self.timeout))?
    }

    async fn login_exchange(&self, username: &str) -> HarnessResult<LegacyLogin> {
        let mut stream = TcpStream::connect(self.addr).await?;
        let mut handshake = vec![HANDSHAKE, self.protocol];
        handshake.extend_from_slice(&string16(username));
        handshake.extend_from_slice(&string16(&self.hostname));
        handshake.extend_from_slice(&i32::from(self.port).to_be_bytes());
        stream.write_all(&handshake).await?;
        stream.flush().await?;

        let mut first = [0u8; 1];
        stream.read_exact(&mut first).await?;
        if first[0] == KICK {
            return Ok(LegacyLogin::Kicked(read_string16(&mut stream).await?));
        }
        Ok(LegacyLogin::Forwarded(LegacySession {
            username: username.to_string(),
            stream,
        }))
    }

    pub async fn ping(&self) -> HarnessResult<LegacyPing> {
        let mut data = vec![self.protocol];
        data.extend_from_slice(&string16(&self.hostname));
        data.extend_from_slice(&i32::from(self.port).to_be_bytes());
        let mut request = vec![PING, 0x01, 0xFA];
        request.extend_from_slice(&string16(PING_CHANNEL));
        let length = u16::try_from(data.len())
            .map_err(|_| HarnessError::setup("legacy ping hostname too long"))?;
        request.extend_from_slice(&length.to_be_bytes());
        request.extend_from_slice(&data);
        self.exchange_ping(&request).await
    }

    pub async fn ping_beta(&self) -> HarnessResult<LegacyPing> {
        self.exchange_ping(&[PING]).await
    }

    async fn exchange_ping(&self, request: &[u8]) -> HarnessResult<LegacyPing> {
        tokio::time::timeout(self.timeout, async {
            let mut stream = TcpStream::connect(self.addr).await?;
            stream.write_all(request).await?;
            stream.flush().await?;
            let mut first = [0u8; 1];
            stream.read_exact(&mut first).await?;
            if first[0] != KICK {
                return Err(HarnessError::Unexpected(format!(
                    "legacy ping answered with 0x{:02X}",
                    first[0]
                )));
            }
            LegacyPing::parse(&read_string16(&mut stream).await?)
        })
        .await
        .map_err(|_| HarnessError::timeout("a legacy ping reply", self.timeout))?
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyHandshake {
    pub protocol: u8,
    pub username: String,
    pub hostname: String,
    pub port: i32,
}

#[derive(Debug)]
pub struct LegacyBackendConn {
    handshake: LegacyHandshake,
    stream: TcpStream,
}

impl LegacyBackendConn {
    pub const fn handshake(&self) -> &LegacyHandshake {
        &self.handshake
    }

    pub fn username(&self) -> &str {
        &self.handshake.username
    }

    pub async fn closed(&mut self, timeout: Duration) -> HarnessResult<()> {
        read_to_end(
            &mut self.stream,
            timeout,
            "the proxy to close the legacy backend",
        )
        .await
    }

    pub async fn close(mut self) {
        let _ = self.stream.shutdown().await;
    }
}

pub struct FakeLegacyBackend {
    addr: SocketAddr,
    connections: tokio::sync::Mutex<mpsc::UnboundedReceiver<LegacyBackendConn>>,
    accept_task: JoinHandle<()>,
}

impl std::fmt::Debug for FakeLegacyBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeLegacyBackend")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl FakeLegacyBackend {
    pub async fn spawn(ping: LegacyPing) -> HarnessResult<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let (conn_tx, conn_rx) = mpsc::unbounded_channel();
        let accept_task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let conn_tx = conn_tx.clone();
                let ping = ping.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_legacy(stream, &ping, &conn_tx).await {
                        eprintln!("fake legacy backend connection failed: {e}");
                    }
                });
            }
        });
        Ok(Self {
            addr,
            connections: tokio::sync::Mutex::new(conn_rx),
            accept_task,
        })
    }

    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn next_connection(&self, timeout: Duration) -> HarnessResult<LegacyBackendConn> {
        let mut connections = self.connections.lock().await;
        tokio::time::timeout(timeout, connections.recv())
            .await
            .map_err(|_| HarnessError::timeout("a legacy backend connection", timeout))?
            .ok_or_else(|| HarnessError::Closed("a legacy backend connection".to_string()))
    }
}

impl Drop for FakeLegacyBackend {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

async fn serve_legacy(
    mut stream: TcpStream,
    ping: &LegacyPing,
    conn_tx: &mpsc::UnboundedSender<LegacyBackendConn>,
) -> HarnessResult<()> {
    let mut first = [0u8; 1];
    stream.read_exact(&mut first).await?;
    match first[0] {
        PING => {
            let mut header = [0u8; 2];
            stream.read_exact(&mut header).await?;
            read_string16(&mut stream).await?;
            let mut length = [0u8; 2];
            stream.read_exact(&mut length).await?;
            let mut data = vec![0u8; usize::from(u16::from_be_bytes(length))];
            stream.read_exact(&mut data).await?;
            stream.write_all(&ping.encode_v1_4()).await?;
            stream.flush().await?;
            Ok(())
        }
        HANDSHAKE => {
            let mut protocol = [0u8; 1];
            stream.read_exact(&mut protocol).await?;
            let username = read_string16(&mut stream).await?;
            let hostname = read_string16(&mut stream).await?;
            let mut port = [0u8; 4];
            stream.read_exact(&mut port).await?;
            let mut greeting = vec![ENCRYPTION_REQUEST];
            greeting.extend_from_slice(&string16("-"));
            greeting.extend_from_slice(&[0, 0, 0, 0]);
            stream.write_all(&greeting).await?;
            stream.flush().await?;
            conn_tx
                .send(LegacyBackendConn {
                    handshake: LegacyHandshake {
                        protocol: protocol[0],
                        username,
                        hostname,
                        port: i32::from_be_bytes(port),
                    },
                    stream,
                })
                .map_err(|_| HarnessError::Closed("the fake legacy backend".to_string()))
        }
        other => Err(HarnessError::Unexpected(format!(
            "legacy backend got first byte 0x{other:02X}"
        ))),
    }
}

pub fn string16(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let count = u16::try_from(units.len()).unwrap_or(u16::MAX);
    let mut out = count.to_be_bytes().to_vec();
    for unit in units.iter().take(usize::from(count)) {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

async fn read_string16(stream: &mut TcpStream) -> HarnessResult<String> {
    let mut length = [0u8; 2];
    stream.read_exact(&mut length).await?;
    let mut bytes = vec![0u8; usize::from(u16::from_be_bytes(length)) * 2];
    stream.read_exact(&mut bytes).await?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_be_bytes(*pair))
        .collect();
    String::from_utf16(&units)
        .map_err(|e| HarnessError::Unexpected(format!("legacy string is not UTF-16: {e}")))
}

async fn read_to_end(stream: &mut TcpStream, timeout: Duration, what: &str) -> HarnessResult<()> {
    let mut sink = Vec::new();
    match tokio::time::timeout(timeout, stream.read_to_end(&mut sink)).await {
        Ok(Ok(_) | Err(_)) => Ok(()),
        Err(_) => Err(HarnessError::timeout(what, timeout)),
    }
}
