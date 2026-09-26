use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_core::auth::mojang::minecraft_server_hash;
use infrarust_protocol::codec::{McBufReadExt, VarInt};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::config::{
    CConfigDisconnect, CFinishConfig, CKnownPacks, SAcknowledgeFinishConfig, SKnownPacks,
};
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::login::{
    CEncryptionRequest, CLoginDisconnect, CLoginPluginRequest, CLoginSuccess, CSetCompression,
    EncryptionProof, SEncryptionResponse, SLoginAcknowledged, SLoginPluginResponse, SLoginStart,
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
use infrarust_protocol::packets::resource_pack::ResourcePackResult;
use infrarust_protocol::packets::status::{
    CPingResponse, CStatusResponse, SPingRequest, SStatusRequest,
};
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use rand::RngCore;
use rand::rngs::OsRng;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use uuid::Uuid;

use crate::DEFAULT_TIMEOUT;
use crate::chat::{self, now_millis};
use crate::client_replies::{ClientReplies, CookieJar};
use crate::error::{HarnessError, HarnessResult};
use crate::framing::{FrameReader, FrameWriter, FramedConn};
use crate::plugin_message::ClientHello;
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
    proxy_source: Option<SocketAddr>,
    claimed_uuid: Option<Uuid>,
    hello: Option<ClientHello>,
    replies: ClientReplies,
    acks_configuration: bool,
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
            proxy_source: None,
            claimed_uuid: None,
            hello: None,
            replies: ClientReplies::default(),
            acks_configuration: true,
        }
    }

    #[must_use]
    pub const fn hold_configuration_ack(mut self) -> Self {
        self.acks_configuration = false;
        self
    }

    #[must_use]
    pub fn cookies(mut self, jar: CookieJar) -> Self {
        self.replies.jar = jar;
        self
    }

    #[must_use]
    pub fn answer_resource_packs(
        mut self,
        results: impl IntoIterator<Item = ResourcePackResult>,
    ) -> Self {
        self.replies.pack_results = results.into_iter().collect();
        self
    }

    #[must_use]
    pub fn hello(mut self, hello: ClientHello) -> Self {
        self.hello = Some(hello);
        self
    }

    #[must_use]
    pub const fn proxy_protocol(mut self, source: SocketAddr) -> Self {
        self.proxy_source = Some(source);
        self
    }

    #[must_use]
    pub const fn claimed_uuid(mut self, uuid: Uuid) -> Self {
        self.claimed_uuid = Some(uuid);
        self
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
        let mut stream = TcpStream::connect(self.addr).await?;
        if let Some(source) = self.proxy_source {
            let header = proxy_v2_header(source, self.addr)?;
            tokio::io::AsyncWriteExt::write_all(&mut stream, &header).await?;
        }
        let mut conn = FramedConn::new(stream)?;
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
                .then(|| self.claimed_uuid.unwrap_or_else(|| offline_uuid(username))),
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
            hello: self.hello.clone(),
            replies: self.replies.clone(),
            acks_configuration: self.acks_configuration,
        };
        let driver = TaskGuard(tokio::spawn(driver.run()));

        let deadline = Instant::now() + self.timeout;
        let mut profile = None;
        let mut profile_name = None;
        let mut server_hash = None;
        let mut config_frames = Vec::new();
        loop {
            let event = tokio::time::timeout_at(deadline, events.recv())
                .await
                .map_err(|_| HarnessError::timeout("the login to complete", self.timeout))?
                .ok_or_else(|| HarnessError::Closed("the login to complete".to_string()))?;
            match event {
                ClientEvent::Encrypted(hash) => server_hash = Some(hash),
                ClientEvent::LoginSucceeded(uuid, name) => {
                    profile = Some(uuid);
                    profile_name = Some(name);
                }
                ClientEvent::Joined(join) => {
                    return Ok(LoginOutcome::Joined(ClientSession {
                        version,
                        username: username.to_string(),
                        uuid: profile,
                        profile_name,
                        server_hash,
                        join,
                        config_frames,
                        writer,
                        events,
                        jar: self.replies.jar.clone(),
                        _driver: driver,
                    }));
                }
                ClientEvent::Disconnected(info) => return Ok(LoginOutcome::Disconnected(info)),
                ClientEvent::Closed(reason) => {
                    return Err(HarnessError::Closed(format!(
                        "the login to complete ({reason})"
                    )));
                }
                ClientEvent::Config(frame) => config_frames.push(frame),
                ClientEvent::Frame(_) => {}
            }
        }
    }
}

#[derive(Debug)]
enum ClientEvent {
    Encrypted(String),
    LoginSucceeded(Uuid, String),
    Joined(PacketFrame),
    Config(PacketFrame),
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
    hello: Option<ClientHello>,
    replies: ClientReplies,
    acks_configuration: bool,
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

    async fn greet(&mut self, state: ConnectionState) -> HarnessResult<()> {
        let Some(hello) = self.hello.take() else {
            return Ok(());
        };
        let mut writer = self.writer.lock().await;
        for frame in hello.frames(state, self.version)? {
            writer.write_frame(&frame).await?;
        }
        Ok(())
    }

    async fn reply(&self, frame: &PacketFrame) -> HarnessResult<()> {
        let answers = self.replies.answer(frame, self.state, self.version)?;
        if answers.is_empty() {
            return Ok(());
        }
        let mut writer = self.writer.lock().await;
        for answer in &answers {
            writer.write_frame(answer).await?;
        }
        Ok(())
    }

    async fn drive(&mut self) -> HarnessResult<()> {
        while let Some(frame) = self.reader.read_frame().await? {
            self.reply(&frame).await?;
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
            let request = wire::decode::<CEncryptionRequest>(&frame, version)?;
            self.answer_encryption(&request).await?;
            return Ok(Flow::Continue);
        }
        if wire::is::<CLoginSuccess>(&frame, version) {
            let success = wire::decode::<CLoginSuccess>(&frame, version)?;
            self.emit(ClientEvent::LoginSucceeded(success.uuid, success.username));
            if version.no_less_than(ProtocolVersion::V1_20_2) {
                self.send(&SLoginAcknowledged).await?;
                self.state = ConnectionState::Config;
                self.greet(ConnectionState::Config).await?;
            } else {
                self.state = ConnectionState::Play;
            }
        }
        Ok(Flow::Continue)
    }

    async fn answer_encryption(&mut self, request: &CEncryptionRequest) -> HarnessResult<()> {
        let key = RsaPublicKey::from_public_key_der(&request.public_key)
            .map_err(|e| HarnessError::Unexpected(format!("server public key: {e}")))?;
        let mut secret = [0u8; 16];
        OsRng.fill_bytes(&mut secret);
        let encrypt = |data: &[u8]| {
            key.encrypt(&mut OsRng, Pkcs1v15Encrypt, data)
                .map_err(|e| HarnessError::Unexpected(format!("RSA encryption failed: {e}")))
        };
        let response = SEncryptionResponse {
            shared_secret: encrypt(&secret)?,
            proof: EncryptionProof::VerifyToken(encrypt(&request.verify_token)?),
        };
        let frame = wire::encode(&response, self.version)?;
        {
            let mut writer = self.writer.lock().await;
            writer.write_frame(&frame).await?;
            writer.enable_encryption(&secret);
        }
        self.reader.enable_decryption(&secret);
        self.emit(ClientEvent::Encrypted(minecraft_server_hash(
            &request.server_id,
            &secret,
            &request.public_key,
        )));
        Ok(())
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
        self.emit(ClientEvent::Config(frame));
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
            if self.acks_configuration {
                self.send(&SAcknowledgeConfiguration).await?;
            }
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
            self.greet(ConnectionState::Play).await?;
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
    profile_name: Option<String>,
    server_hash: Option<String>,
    join: PacketFrame,
    config_frames: Vec<PacketFrame>,
    writer: Arc<Mutex<FrameWriter>>,
    events: mpsc::UnboundedReceiver<ClientEvent>,
    jar: CookieJar,
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

    pub fn profile_name(&self) -> Option<&str> {
        self.profile_name.as_deref()
    }

    pub fn server_hash(&self) -> Option<&str> {
        self.server_hash.as_deref()
    }

    pub const fn join_frame(&self) -> &PacketFrame {
        &self.join
    }

    pub fn config_frames(&self) -> &[PacketFrame] {
        &self.config_frames
    }

    pub fn cookies(&self) -> CookieJar {
        self.jar.clone()
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
            timestamp: now_millis(),
            ..SChatMessage::default()
        };
        self.send_packet(&packet).await
    }

    pub async fn chat_signed(&self, message: &str, offset: i32) -> HarnessResult<PacketFrame> {
        let frame = wire::encode(
            &chat::signed_chat(message, offset, self.version),
            self.version,
        )?;
        self.send_frame(&frame).await?;
        Ok(frame)
    }

    pub async fn command_signed(&self, command: &str, offset: i32) -> HarnessResult<PacketFrame> {
        let frame = chat::signed_command_frame(command, offset, self.version)?;
        self.send_frame(&frame).await?;
        Ok(frame)
    }

    pub async fn command(&self, command: &str) -> HarnessResult<()> {
        let command = command.strip_prefix('/').unwrap_or(command);
        if self.version.less_than(ProtocolVersion::V1_19) {
            return self.chat(&format!("/{command}")).await;
        }
        let packet = SChatCommand {
            command: command.to_string(),
            timestamp: now_millis(),
            ..SChatCommand::default()
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
                ClientEvent::Encrypted(_)
                | ClientEvent::LoginSucceeded(..)
                | ClientEvent::Joined(_)
                | ClientEvent::Config(_)
                | ClientEvent::Frame(_) => {}
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

    pub async fn expect_config<P: Packet>(&mut self, timeout: Duration) -> HarnessResult<P> {
        let version = self.version;
        self.wait_for(P::NAME, timeout, |event| match event {
            ClientEvent::Config(frame) if wire::is::<P>(frame, version) => {
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

    pub async fn close_write(&self) -> HarnessResult<()> {
        self.writer.lock().await.shutdown().await
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

fn proxy_v2_header(source: SocketAddr, destination: SocketAddr) -> HarnessResult<Vec<u8>> {
    const SIGNATURE: [u8; 12] = [
        0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A,
    ];
    let (SocketAddr::V4(source), SocketAddr::V4(destination)) = (source, destination) else {
        return Err(HarnessError::setup(
            "the PROXY protocol header supports IPv4 addresses only",
        ));
    };
    let mut header = SIGNATURE.to_vec();
    header.extend_from_slice(&[0x21, 0x11, 0x00, 0x0C]);
    header.extend_from_slice(&source.ip().octets());
    header.extend_from_slice(&destination.ip().octets());
    header.extend_from_slice(&source.port().to_be_bytes());
    header.extend_from_slice(&destination.port().to_be_bytes());
    Ok(header)
}
