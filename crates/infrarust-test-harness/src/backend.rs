use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_protocol::codec::{McBufReadExt, VarInt};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::config::{CFinishConfig, SAcknowledgeFinishConfig};
use infrarust_protocol::packets::login::{
    CLoginDisconnect, CLoginSuccess, CSetCompression, SLoginAcknowledged, SLoginStart,
};
use infrarust_protocol::packets::play::chat::{CChatMessageLegacy, CSystemChatMessage};
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::packets::status::{
    CPingResponse, CStatusResponse, SPingRequest, SStatusRequest,
};
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::Instant;
use uuid::Uuid;

use crate::error::{HarnessError, HarnessResult};
use crate::framing::{FrameReader, FrameWriter, FramedConn};
use crate::text::encode_component_json;
use crate::wire;

#[derive(Debug, Clone)]
pub enum LoginBehavior {
    Accept,
    Refuse(Value),
    Hang,
}

impl LoginBehavior {
    pub fn refuse_text(text: &str) -> Self {
        Self::Refuse(json!({ "text": text }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedHandshake {
    pub protocol_version: i32,
    pub server_address: String,
    pub server_port: u16,
    pub next_state: i32,
}

impl ObservedHandshake {
    fn parse(frame: &PacketFrame) -> HarnessResult<Self> {
        if frame.id != 0x00 {
            return Err(HarnessError::Unexpected(format!(
                "expected a handshake, got packet 0x{:02X}",
                frame.id
            )));
        }
        let mut r = frame.payload.as_ref();
        Ok(Self {
            protocol_version: r.read_var_int()?.0,
            server_address: r.read_string()?,
            server_port: r.read_u16_be()?,
            next_state: r.read_var_int()?.0,
        })
    }

    pub const fn version(&self) -> ProtocolVersion {
        ProtocolVersion(self.protocol_version)
    }
}

#[derive(Debug, Clone)]
struct BackendConfig {
    compression: Option<i32>,
    login: LoginBehavior,
    status: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct FakeBackendBuilder {
    config: BackendConfig,
}

impl FakeBackendBuilder {
    #[must_use]
    pub const fn compression(mut self, threshold: Option<i32>) -> Self {
        self.config.compression = threshold;
        self
    }

    #[must_use]
    pub fn login(mut self, behavior: LoginBehavior) -> Self {
        self.config.login = behavior;
        self
    }

    #[must_use]
    pub fn status(mut self, json: Value) -> Self {
        self.config.status = Some(json);
        self
    }

    pub async fn spawn(self) -> HarnessResult<FakeBackend> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let (conn_tx, conn_rx) = mpsc::unbounded_channel();
        let status_requests = Arc::new(AtomicUsize::new(0));
        let accept_task = tokio::spawn(accept_loop(
            listener,
            Arc::new(self.config),
            conn_tx,
            Arc::clone(&status_requests),
        ));
        Ok(FakeBackend {
            addr,
            connections: tokio::sync::Mutex::new(conn_rx),
            status_requests,
            accept_task,
        })
    }
}

pub struct FakeBackend {
    addr: SocketAddr,
    connections: tokio::sync::Mutex<mpsc::UnboundedReceiver<BackendConn>>,
    status_requests: Arc<AtomicUsize>,
    accept_task: JoinHandle<()>,
}

impl FakeBackend {
    pub fn builder() -> FakeBackendBuilder {
        FakeBackendBuilder {
            config: BackendConfig {
                compression: None,
                login: LoginBehavior::Accept,
                status: None,
            },
        }
    }

    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn status_requests(&self) -> usize {
        self.status_requests.load(Ordering::SeqCst)
    }

    pub async fn next_connection(&self, timeout: Duration) -> HarnessResult<BackendConn> {
        let mut connections = self.connections.lock().await;
        tokio::time::timeout(timeout, connections.recv())
            .await
            .map_err(|_| HarnessError::timeout("a backend connection", timeout))?
            .ok_or_else(|| HarnessError::Closed("a backend connection".to_string()))
    }
}

impl Drop for FakeBackend {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

async fn accept_loop(
    listener: TcpListener,
    config: Arc<BackendConfig>,
    conn_tx: mpsc::UnboundedSender<BackendConn>,
    status_requests: Arc<AtomicUsize>,
) {
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let config = Arc::clone(&config);
                let conn_tx = conn_tx.clone();
                let status_requests = Arc::clone(&status_requests);
                tasks.spawn(async move {
                    if let Err(e) = serve(stream, &config, &conn_tx, &status_requests).await {
                        eprintln!("fake backend connection failed: {e}");
                    }
                });
            }
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
        }
    }
}

async fn serve(
    stream: TcpStream,
    config: &BackendConfig,
    conn_tx: &mpsc::UnboundedSender<BackendConn>,
    status_requests: &AtomicUsize,
) -> HarnessResult<()> {
    let mut conn = FramedConn::new(stream)?;
    let handshake = ObservedHandshake::parse(&read_required(&mut conn, "handshake").await?)?;
    match handshake.next_state {
        1 => {
            status_requests.fetch_add(1, Ordering::SeqCst);
            serve_status(conn, &handshake, config).await
        }
        2 | 3 => {
            let backend_conn = serve_login(conn, handshake, config).await?;
            conn_tx
                .send(backend_conn)
                .map_err(|_| HarnessError::Closed("the fake backend".to_string()))
        }
        other => Err(HarnessError::Unexpected(format!(
            "handshake next_state {other}"
        ))),
    }
}

async fn read_required(conn: &mut FramedConn, what: &str) -> HarnessResult<PacketFrame> {
    conn.read_frame()
        .await?
        .ok_or_else(|| HarnessError::Closed(what.to_string()))
}

async fn send<P: Packet>(
    conn: &mut FramedConn,
    packet: &P,
    version: ProtocolVersion,
) -> HarnessResult<()> {
    conn.write_frame(&wire::encode(packet, version)?).await
}

async fn serve_status(
    mut conn: FramedConn,
    handshake: &ObservedHandshake,
    config: &BackendConfig,
) -> HarnessResult<()> {
    let version = handshake.version();
    wire::decode::<SStatusRequest>(&read_required(&mut conn, "status request").await?, version)?;
    let json = config.status.clone().unwrap_or_else(|| {
        json!({
            "version": { "name": "FakeBackend", "protocol": handshake.protocol_version },
            "players": { "max": 20, "online": 0, "sample": [] },
            "description": { "text": "Infrarust fake backend" },
        })
    });
    let response = CStatusResponse {
        json_response: json.to_string(),
    };
    send(&mut conn, &response, version).await?;
    let Some(frame) = conn.read_frame().await? else {
        return Ok(());
    };
    let ping = wire::decode::<SPingRequest>(&frame, version)?;
    send(
        &mut conn,
        &CPingResponse {
            payload: ping.payload,
        },
        version,
    )
    .await
}

async fn serve_login(
    mut conn: FramedConn,
    handshake: ObservedHandshake,
    config: &BackendConfig,
) -> HarnessResult<BackendConn> {
    let version = handshake.version();
    let start =
        wire::decode::<SLoginStart>(&read_required(&mut conn, "login start").await?, version)?;
    let mut setup = ConnSetup {
        handshake,
        version,
        username: start.name,
        uuid: start.uuid,
        state: ConnectionState::Login,
        config_frames: Vec::new(),
    };

    match &config.login {
        LoginBehavior::Refuse(reason) => {
            let refuse = CLoginDisconnect {
                reason: reason.to_string(),
            };
            send(&mut conn, &refuse, version).await?;
            return Ok(BackendConn::start(conn, setup));
        }
        LoginBehavior::Hang => return Ok(BackendConn::start(conn, setup)),
        LoginBehavior::Accept => {}
    }

    if let Some(threshold) = config.compression
        && version.no_less_than(ProtocolVersion::V1_8)
    {
        let packet = CSetCompression {
            threshold: VarInt(threshold),
        };
        send(&mut conn, &packet, version).await?;
        conn.set_compression(Some(threshold));
    }

    let success = CLoginSuccess {
        uuid: setup
            .uuid
            .filter(|uuid| !uuid.is_nil())
            .unwrap_or_else(|| offline_uuid(&setup.username)),
        username: setup.username.clone(),
        properties: Vec::new(),
        strict_error_handling: false,
        session_id: version
            .no_less_than(ProtocolVersion::V26_2)
            .then(Uuid::new_v4),
    };
    send(&mut conn, &success, version).await?;

    if version.no_less_than(ProtocolVersion::V1_20_2) {
        read_until::<SLoginAcknowledged>(&mut conn, version, &mut setup.config_frames).await?;
        setup.state = ConnectionState::Config;
        send(&mut conn, &CFinishConfig, version).await?;
        read_until::<SAcknowledgeFinishConfig>(&mut conn, version, &mut setup.config_frames)
            .await?;
    }

    setup.state = ConnectionState::Play;
    conn.write_frame(&infrarust_core::test_support::join_game_frame(version)?)
        .await?;
    Ok(BackendConn::start(conn, setup))
}

async fn read_until<P: Packet>(
    conn: &mut FramedConn,
    version: ProtocolVersion,
    skipped: &mut Vec<PacketFrame>,
) -> HarnessResult<()> {
    loop {
        let frame = read_required(conn, P::NAME).await?;
        if wire::is::<P>(&frame, version) {
            return Ok(());
        }
        skipped.push(frame);
    }
}

struct ConnSetup {
    handshake: ObservedHandshake,
    version: ProtocolVersion,
    username: String,
    uuid: Option<Uuid>,
    state: ConnectionState,
    config_frames: Vec<PacketFrame>,
}

pub struct BackendConn {
    setup: ConnSetup,
    peer: Option<SocketAddr>,
    writer: Option<FrameWriter>,
    frames: mpsc::UnboundedReceiver<PacketFrame>,
    received: Arc<Mutex<Vec<PacketFrame>>>,
    closed: watch::Receiver<bool>,
    reader_task: JoinHandle<()>,
}

impl BackendConn {
    fn start(conn: FramedConn, setup: ConnSetup) -> Self {
        let peer = conn.peer_addr().ok();
        let (reader, writer) = conn.into_split();
        let (frame_tx, frames) = mpsc::unbounded_channel();
        let (closed_tx, closed) = watch::channel(false);
        let received = Arc::new(Mutex::new(Vec::new()));
        let reader_task = tokio::spawn(record_frames(
            reader,
            frame_tx,
            Arc::clone(&received),
            closed_tx,
        ));
        Self {
            setup,
            peer,
            writer: Some(writer),
            frames,
            received,
            closed,
            reader_task,
        }
    }

    pub const fn handshake(&self) -> &ObservedHandshake {
        &self.setup.handshake
    }

    pub const fn version(&self) -> ProtocolVersion {
        self.setup.version
    }

    pub fn username(&self) -> &str {
        &self.setup.username
    }

    pub const fn uuid(&self) -> Option<Uuid> {
        self.setup.uuid
    }

    pub const fn state(&self) -> ConnectionState {
        self.setup.state
    }

    pub const fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer
    }

    pub fn config_frames(&self) -> &[PacketFrame] {
        &self.setup.config_frames
    }

    pub fn received(&self) -> Vec<PacketFrame> {
        self.received
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub async fn recv_frame(&mut self, timeout: Duration) -> HarnessResult<PacketFrame> {
        tokio::time::timeout(timeout, self.frames.recv())
            .await
            .map_err(|_| HarnessError::timeout("a frame from the client", timeout))?
            .ok_or_else(|| HarnessError::Closed("a frame from the client".to_string()))
    }

    pub async fn expect<P: Packet>(&mut self, timeout: Duration) -> HarnessResult<P> {
        let version = self.setup.version;
        let deadline = Instant::now() + timeout;
        loop {
            let frame = tokio::time::timeout_at(deadline, self.frames.recv())
                .await
                .map_err(|_| HarnessError::timeout(P::NAME, timeout))?
                .ok_or_else(|| HarnessError::Closed(P::NAME.to_string()))?;
            if wire::is::<P>(&frame, version) {
                return wire::decode::<P>(&frame, version);
            }
        }
    }

    pub async fn send_frame(&mut self, frame: &PacketFrame) -> HarnessResult<()> {
        self.writer
            .as_mut()
            .ok_or_else(|| HarnessError::Closed("sending on a closed backend connection".into()))?
            .write_frame(frame)
            .await
    }

    pub async fn send_packet<P: Packet>(&mut self, packet: &P) -> HarnessResult<()> {
        let frame = wire::encode(packet, self.setup.version)?;
        self.send_frame(&frame).await
    }

    pub async fn kick_json(&mut self, json: &str) -> HarnessResult<()> {
        if self.setup.state == ConnectionState::Login {
            let packet = CLoginDisconnect {
                reason: json.to_string(),
            };
            self.send_packet(&packet).await?;
        } else {
            let reason = encode_component_json(json, self.setup.version)?;
            self.send_packet(&CDisconnect { reason }).await?;
        }
        self.close().await;
        Ok(())
    }

    pub async fn kick_raw(&mut self, reason: Vec<u8>) -> HarnessResult<()> {
        self.send_packet(&CDisconnect { reason }).await?;
        self.close().await;
        Ok(())
    }

    pub async fn send_system_message_json(&mut self, json: &str) -> HarnessResult<()> {
        let version = self.setup.version;
        if version.no_less_than(ProtocolVersion::V1_19) {
            let packet = CSystemChatMessage {
                content: encode_component_json(json, version)?,
                overlay: false,
            };
            self.send_packet(&packet).await
        } else {
            let packet = CChatMessageLegacy {
                content: json.to_string(),
                position: 1,
            };
            self.send_packet(&packet).await
        }
    }

    pub async fn close(&mut self) {
        if let Some(mut writer) = self.writer.take() {
            let _ = writer.shutdown().await;
        }
    }

    pub fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }

    pub async fn closed(&self, timeout: Duration) -> HarnessResult<()> {
        let mut closed = self.closed.clone();
        match tokio::time::timeout(timeout, closed.wait_for(|c| *c)).await {
            Ok(_) => Ok(()),
            Err(_) => Err(HarnessError::timeout(
                "the client to close the backend connection",
                timeout,
            )),
        }
    }
}

impl Drop for BackendConn {
    fn drop(&mut self) {
        self.reader_task.abort();
    }
}

async fn record_frames(
    mut reader: FrameReader,
    frame_tx: mpsc::UnboundedSender<PacketFrame>,
    received: Arc<Mutex<Vec<PacketFrame>>>,
    closed_tx: watch::Sender<bool>,
) {
    while let Ok(Some(frame)) = reader.read_frame().await {
        received
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(frame.clone());
        let _ = frame_tx.send(frame);
    }
    closed_tx.send_replace(true);
}
