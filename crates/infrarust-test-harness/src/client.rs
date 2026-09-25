use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_protocol::codec::{McBufReadExt, McBufWriteExt, VarInt};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::config::{
    CConfigDisconnect, CFinishConfig, CKnownPacks, SAcknowledgeFinishConfig, SKnownPacks,
};
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::login::{
    CEncryptionRequest, CLoginDisconnect, CLoginPluginRequest, CLoginSuccess, CSetCompression,
    SLoginAcknowledged, SLoginPluginResponse, SLoginStart,
};
use infrarust_protocol::packets::play::chat::{
    CChatMessageLegacy, CSystemChatMessage, SChatCommand, SChatMessage,
};
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::packets::play::join_game::CJoinGame;
use infrarust_protocol::packets::play::keepalive::{CKeepAlive, SKeepAlive};
use infrarust_protocol::packets::play::start_configuration::{
    CStartConfiguration, SAcknowledgeConfiguration,
};
use infrarust_protocol::packets::status::{
    CPingResponse, CStatusResponse, SPingRequest, SStatusRequest,
};
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use serde_json::Value;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use uuid::Uuid;

use crate::DEFAULT_TIMEOUT;
use crate::error::{HarnessError, HarnessResult};
use crate::framing::{FrameReader, FrameWriter, FramedConn};
use crate::text::{DisconnectInfo, component_text, uses_nbt_components};
use crate::wire;

const STATUS_PING_PAYLOAD: i64 = 0x0049_6E66_7261;

#[derive(Debug, Clone)]
pub struct FakeClient {
    addr: SocketAddr,
    version: ProtocolVersion,
    domain: String,
    port: u16,
    timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct StatusResult {
    pub json: Value,
    pub raw: String,
}

#[derive(Debug)]
pub enum LoginOutcome {
    Joined(ClientSession),
    Disconnected(DisconnectInfo),
}

impl LoginOutcome {
    pub fn joined(self) -> HarnessResult<ClientSession> {
        match self {
            Self::Joined(session) => Ok(session),
            Self::Disconnected(info) => Err(HarnessError::Disconnected(Box::new(info))),
        }
    }

    pub fn disconnected(self) -> HarnessResult<DisconnectInfo> {
        match self {
            Self::Disconnected(info) => Ok(info),
            Self::Joined(session) => Err(HarnessError::Unexpected(format!(
                "expected {} to be disconnected, but it joined",
                session.username
            ))),
        }
    }
}

impl FakeClient {
    pub fn new(addr: SocketAddr, version: ProtocolVersion) -> Self {
        Self {
            addr,
            version,
            domain: addr.ip().to_string(),
            port: addr.port(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = domain.into();
        self
    }

    #[must_use]
    pub const fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub const fn version(&self) -> ProtocolVersion {
        self.version
    }

    async fn open(&self, next_state: ConnectionState) -> HarnessResult<FramedConn> {
        let mut conn = FramedConn::connect(self.addr).await?;
        let handshake = SHandshake {
            protocol_version: VarInt(self.version.0),
            server_address: self.domain.clone(),
            server_port: self.port,
            next_state,
        };
        conn.write_frame(&wire::encode(&handshake, self.version)?)
            .await?;
        Ok(conn)
    }

    pub async fn status(&self) -> HarnessResult<StatusResult> {
        tokio::time::timeout(self.timeout, self.status_exchange())
            .await
            .map_err(|_| HarnessError::timeout("a status exchange", self.timeout))?
    }

    async fn status_exchange(&self) -> HarnessResult<StatusResult> {
        let version = self.version;
        let mut conn = self.open(ConnectionState::Status).await?;
        conn.write_frame(&wire::encode(&SStatusRequest, version)?)
            .await?;
        let frame = conn
            .read_frame()
            .await?
            .ok_or_else(|| HarnessError::Closed("the status response".to_string()))?;
        let response = wire::decode::<CStatusResponse>(&frame, version)?;
        let json = serde_json::from_str(&response.json_response)
            .map_err(|e| HarnessError::Unexpected(format!("status JSON: {e}")))?;
        let ping = SPingRequest {
            payload: STATUS_PING_PAYLOAD,
        };
        conn.write_frame(&wire::encode(&ping, version)?).await?;
        let frame = conn
            .read_frame()
            .await?
            .ok_or_else(|| HarnessError::Closed("the status pong".to_string()))?;
        let pong = wire::decode::<CPingResponse>(&frame, version)?;
        if pong.payload != STATUS_PING_PAYLOAD {
            return Err(HarnessError::Unexpected(format!(
                "pong payload {} does not echo the ping",
                pong.payload
            )));
        }
        Ok(StatusResult {
            json,
            raw: response.json_response,
        })
    }

    pub async fn login(&self, username: &str) -> HarnessResult<LoginOutcome> {
        let version = self.version;
        let mut conn = self.open(ConnectionState::Login).await?;
        let start = SLoginStart {
            name: username.to_string(),
            uuid: version
                .no_less_than(ProtocolVersion::V1_19_1)
                .then(|| offline_uuid(username)),
            profile_key: None,
        };
        conn.write_frame(&wire::encode(&start, version)?).await?;

        let (reader, writer) = conn.into_split();
        let writer = Arc::new(Mutex::new(writer));
        let (events_tx, mut events) = mpsc::unbounded_channel();
        let driver = Driver {
            reader,
            writer: Arc::clone(&writer),
            version,
            state: ConnectionState::Login,
            events: events_tx,
        };
        let driver = TaskGuard(tokio::spawn(driver.run()));

        let deadline = Instant::now() + self.timeout;
        let mut profile = None;
        loop {
            let event = tokio::time::timeout_at(deadline, events.recv())
                .await
                .map_err(|_| HarnessError::timeout("the login to complete", self.timeout))?
                .ok_or_else(|| HarnessError::Closed("the login to complete".to_string()))?;
            match event {
                ClientEvent::LoginSucceeded(uuid) => profile = Some(uuid),
                ClientEvent::Joined(join) => {
                    return Ok(LoginOutcome::Joined(ClientSession {
                        version,
                        username: username.to_string(),
                        uuid: profile,
                        join,
                        writer,
                        events,
                        _driver: driver,
                    }));
                }
                ClientEvent::Disconnected(info) => return Ok(LoginOutcome::Disconnected(info)),
                ClientEvent::Closed(reason) => {
                    return Err(HarnessError::Closed(format!(
                        "the login to complete ({reason})"
                    )));
                }
                ClientEvent::Frame(_) => {}
            }
        }
    }
}

#[derive(Debug)]
enum ClientEvent {
    LoginSucceeded(Uuid),
    Joined(PacketFrame),
    Frame(PacketFrame),
    Disconnected(DisconnectInfo),
    Closed(String),
}

#[derive(Debug)]
struct TaskGuard(JoinHandle<()>);

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum Flow {
    Continue,
    Stop,
}

struct Driver {
    reader: FrameReader,
    writer: Arc<Mutex<FrameWriter>>,
    version: ProtocolVersion,
    state: ConnectionState,
    events: mpsc::UnboundedSender<ClientEvent>,
}

impl Driver {
    async fn run(mut self) {
        let reason = match self.drive().await {
            Ok(()) => format!("connection ended in {} state", self.state),
            Err(e) => e.to_string(),
        };
        let _ = self.writer.lock().await.shutdown().await;
        self.emit(ClientEvent::Closed(reason));
    }

    fn emit(&self, event: ClientEvent) {
        let _ = self.events.send(event);
    }

    async fn send<P: Packet>(&self, packet: &P) -> HarnessResult<()> {
        let frame = wire::encode(packet, self.version)?;
        self.writer.lock().await.write_frame(&frame).await
    }

    async fn drive(&mut self) -> HarnessResult<()> {
        while let Some(frame) = self.reader.read_frame().await? {
            let flow = match self.state {
                ConnectionState::Login => self.on_login(frame).await?,
                ConnectionState::Config => self.on_config(frame).await?,
                ConnectionState::Play => self.on_play(frame).await?,
                other => {
                    return Err(HarnessError::Unexpected(format!(
                        "client driver cannot run in {other} state"
                    )));
                }
            };
            if matches!(flow, Flow::Stop) {
                break;
            }
        }
        Ok(())
    }

    async fn on_login(&mut self, frame: PacketFrame) -> HarnessResult<Flow> {
        let version = self.version;
        if wire::is::<CLoginDisconnect>(&frame, version) {
            let packet = wire::decode::<CLoginDisconnect>(&frame, version)?;
            self.emit(ClientEvent::Disconnected(DisconnectInfo::from_json_string(
                ConnectionState::Login,
                &packet.reason,
            )));
            return Ok(Flow::Stop);
        }
        if wire::is::<CSetCompression>(&frame, version) {
            let packet = wire::decode::<CSetCompression>(&frame, version)?;
            let threshold = (packet.threshold.0 >= 0).then_some(packet.threshold.0);
            self.reader.set_compression(threshold);
            self.writer.lock().await.set_compression(threshold);
            return Ok(Flow::Continue);
        }
        if wire::is::<CLoginPluginRequest>(&frame, version) {
            let request = wire::decode::<CLoginPluginRequest>(&frame, version)?;
            let response = SLoginPluginResponse {
                message_id: request.message_id,
                successful: false,
                data: Vec::new(),
            };
            self.send(&response).await?;
            return Ok(Flow::Continue);
        }
        if wire::is::<CEncryptionRequest>(&frame, version) {
            return Err(HarnessError::Unsupported(
                "FakeClient does not implement online-mode encryption yet".to_string(),
            ));
        }
        if wire::is::<CLoginSuccess>(&frame, version) {
            let success = wire::decode::<CLoginSuccess>(&frame, version)?;
            self.emit(ClientEvent::LoginSucceeded(success.uuid));
            if version.no_less_than(ProtocolVersion::V1_20_2) {
                self.send(&SLoginAcknowledged).await?;
                self.state = ConnectionState::Config;
            } else {
                self.state = ConnectionState::Play;
            }
        }
        Ok(Flow::Continue)
    }

    async fn on_config(&mut self, frame: PacketFrame) -> HarnessResult<Flow> {
        let version = self.version;
        if wire::is::<CFinishConfig>(&frame, version) {
            self.send(&SAcknowledgeFinishConfig).await?;
            self.state = ConnectionState::Play;
            return Ok(Flow::Continue);
        }
        if wire::is::<CKnownPacks>(&frame, version) {
            let known = wire::decode::<CKnownPacks>(&frame, version)?;
            self.send(&SKnownPacks { packs: known.packs }).await?;
            return Ok(Flow::Continue);
        }
        if wire::is::<CConfigDisconnect>(&frame, version) {
            let packet = wire::decode::<CConfigDisconnect>(&frame, version)?;
            let info = if uses_nbt_components(version) {
                DisconnectInfo::from_component(ConnectionState::Config, packet.reason, version)
            } else {
                let json = packet.reason.as_slice().read_string()?;
                DisconnectInfo::from_json_string(ConnectionState::Config, &json)
            };
            self.emit(ClientEvent::Disconnected(info));
            return Ok(Flow::Stop);
        }
        Ok(Flow::Continue)
    }

    async fn on_play(&mut self, frame: PacketFrame) -> HarnessResult<Flow> {
        let version = self.version;
        if wire::is::<CKeepAlive>(&frame, version) {
            let keep_alive = wire::decode::<CKeepAlive>(&frame, version)?;
            self.send(&SKeepAlive { id: keep_alive.id }).await?;
            return Ok(Flow::Continue);
        }
        if wire::is::<CStartConfiguration>(&frame, version) {
            self.send(&SAcknowledgeConfiguration).await?;
            self.state = ConnectionState::Config;
            return Ok(Flow::Continue);
        }
        if wire::is::<CDisconnect>(&frame, version) {
            let packet = wire::decode::<CDisconnect>(&frame, version)?;
            self.emit(ClientEvent::Disconnected(DisconnectInfo::from_component(
                ConnectionState::Play,
                packet.reason,
                version,
            )));
            return Ok(Flow::Stop);
        }
        if wire::is::<CJoinGame>(&frame, version) {
            self.emit(ClientEvent::Joined(frame));
        } else {
            self.emit(ClientEvent::Frame(frame));
        }
        Ok(Flow::Continue)
    }
}

pub struct ClientSession {
    version: ProtocolVersion,
    username: String,
    uuid: Option<Uuid>,
    join: PacketFrame,
    writer: Arc<Mutex<FrameWriter>>,
    events: mpsc::UnboundedReceiver<ClientEvent>,
    _driver: TaskGuard,
}

impl std::fmt::Debug for ClientSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientSession")
            .field("version", &self.version)
            .field("username", &self.username)
            .field("uuid", &self.uuid)
            .finish_non_exhaustive()
    }
}

impl ClientSession {
    pub const fn version(&self) -> ProtocolVersion {
        self.version
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub const fn uuid(&self) -> Option<Uuid> {
        self.uuid
    }

    pub const fn join_frame(&self) -> &PacketFrame {
        &self.join
    }

    pub async fn send_frame(&self, frame: &PacketFrame) -> HarnessResult<()> {
        self.writer.lock().await.write_frame(frame).await
    }

    pub async fn send_packet<P: Packet>(&self, packet: &P) -> HarnessResult<()> {
        self.send_frame(&wire::encode(packet, self.version)?).await
    }

    pub async fn chat(&self, message: &str) -> HarnessResult<()> {
        let packet = SChatMessage {
            message: message.to_string(),
            remaining: chat_trailer(self.version)?,
        };
        self.send_packet(&packet).await
    }

    pub async fn command(&self, command: &str) -> HarnessResult<()> {
        let command = command.strip_prefix('/').unwrap_or(command);
        if self.version.less_than(ProtocolVersion::V1_19) {
            return self.chat(&format!("/{command}")).await;
        }
        let packet = SChatCommand {
            command: command.to_string(),
            remaining: command_trailer(self.version)?,
        };
        self.send_packet(&packet).await
    }

    async fn wait_for<T>(
        &mut self,
        what: &str,
        timeout: Duration,
        mut pick: impl FnMut(&ClientEvent) -> Option<HarnessResult<T>>,
    ) -> HarnessResult<T> {
        let deadline = Instant::now() + timeout;
        loop {
            let event = tokio::time::timeout_at(deadline, self.events.recv())
                .await
                .map_err(|_| HarnessError::timeout(what, timeout))?
                .ok_or_else(|| HarnessError::Closed(what.to_string()))?;
            if let Some(result) = pick(&event) {
                return result;
            }
            match event {
                ClientEvent::Disconnected(info) => {
                    return Err(HarnessError::Disconnected(Box::new(info)));
                }
                ClientEvent::Closed(reason) => {
                    return Err(HarnessError::Closed(format!("{what} ({reason})")));
                }
                ClientEvent::LoginSucceeded(_) | ClientEvent::Joined(_) | ClientEvent::Frame(_) => {
                }
            }
        }
    }

    pub async fn recv_frame(&mut self, timeout: Duration) -> HarnessResult<PacketFrame> {
        self.wait_for("a play frame", timeout, |event| match event {
            ClientEvent::Frame(frame) | ClientEvent::Joined(frame) => Some(Ok(frame.clone())),
            _ => None,
        })
        .await
    }

    pub async fn expect<P: Packet>(&mut self, timeout: Duration) -> HarnessResult<P> {
        let version = self.version;
        self.wait_for(P::NAME, timeout, |event| match event {
            ClientEvent::Frame(frame) | ClientEvent::Joined(frame)
                if wire::is::<P>(frame, version) =>
            {
                Some(wire::decode::<P>(frame, version))
            }
            _ => None,
        })
        .await
    }

    pub async fn expect_system_message(&mut self, timeout: Duration) -> HarnessResult<Vec<u8>> {
        let version = self.version;
        self.wait_for("a system chat message", timeout, |event| match event {
            ClientEvent::Frame(frame) => system_message_content(frame, version).transpose(),
            _ => None,
        })
        .await
    }

    pub async fn expect_system_text(&mut self, timeout: Duration) -> HarnessResult<String> {
        let raw = self.expect_system_message(timeout).await?;
        Ok(component_text(&raw, self.version))
    }

    pub async fn expect_disconnect(&mut self, timeout: Duration) -> HarnessResult<DisconnectInfo> {
        self.wait_for("a disconnect", timeout, |event| match event {
            ClientEvent::Disconnected(info) => Some(Ok(info.clone())),
            _ => None,
        })
        .await
    }

    pub async fn expect_join(&mut self, timeout: Duration) -> HarnessResult<PacketFrame> {
        self.wait_for("a JoinGame", timeout, |event| match event {
            ClientEvent::Joined(frame) => Some(Ok(frame.clone())),
            _ => None,
        })
        .await
    }

    pub async fn quit(self) {
        let _ = self.writer.lock().await.shutdown().await;
    }
}

fn system_message_content(
    frame: &PacketFrame,
    version: ProtocolVersion,
) -> HarnessResult<Option<Vec<u8>>> {
    if wire::is::<CSystemChatMessage>(frame, version) {
        let message = wire::decode::<CSystemChatMessage>(frame, version)?;
        return Ok((!message.overlay).then_some(message.content));
    }
    if wire::is::<CChatMessageLegacy>(frame, version) {
        let message = wire::decode::<CChatMessageLegacy>(frame, version)?;
        return Ok((message.position != 2).then(|| message.content.into_bytes()));
    }
    Ok(None)
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

fn write_acknowledgements(buf: &mut Vec<u8>, version: ProtocolVersion) -> HarnessResult<()> {
    buf.write_var_int(&VarInt(0))?;
    buf.extend_from_slice(&[0; 3]);
    if version.no_less_than(ProtocolVersion::V1_21_5) {
        buf.write_u8(0)?;
    }
    Ok(())
}

fn chat_trailer(version: ProtocolVersion) -> HarnessResult<Vec<u8>> {
    let mut buf = Vec::new();
    if version.less_than(ProtocolVersion::V1_19) {
        return Ok(buf);
    }
    buf.write_i64_be(now_millis())?;
    buf.write_i64_be(0)?;
    if version.less_than(ProtocolVersion::V1_19_3) {
        buf.write_var_int(&VarInt(0))?;
        buf.write_bool(false)?;
        if version.no_less_than(ProtocolVersion::V1_19_1) {
            buf.write_var_int(&VarInt(0))?;
            buf.write_bool(false)?;
        }
    } else {
        buf.write_bool(false)?;
        write_acknowledgements(&mut buf, version)?;
    }
    Ok(buf)
}

fn command_trailer(version: ProtocolVersion) -> HarnessResult<Vec<u8>> {
    let mut buf = Vec::new();
    if version.no_less_than(ProtocolVersion::V1_20_5) {
        return Ok(buf);
    }
    buf.write_i64_be(now_millis())?;
    buf.write_i64_be(0)?;
    buf.write_var_int(&VarInt(0))?;
    if version.less_than(ProtocolVersion::V1_19_3) {
        buf.write_bool(false)?;
        if version.no_less_than(ProtocolVersion::V1_19_1) {
            buf.write_var_int(&VarInt(0))?;
            buf.write_bool(false)?;
        }
    } else {
        write_acknowledgements(&mut buf, version)?;
    }
    Ok(buf)
}
