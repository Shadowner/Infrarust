use std::time::Duration;

use bytes::Bytes;
use infrarust_protocol::codec::{McBufReadExt, McBufWriteExt};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::config::{
    CConfigPluginMessage, SConfigClientInformation, SConfigPluginMessage,
};
use infrarust_protocol::packets::play::client_information::{
    ClientInformation, SClientInformation,
};
use infrarust_protocol::packets::play::plugin_message::{CPluginMessage, SPluginMessage};
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use tokio::time::Instant;

use crate::backend::BackendConn;
use crate::client::ClientSession;
use crate::error::{HarnessError, HarnessResult};
use crate::wire;

pub const BRAND: &str = "minecraft:brand";
pub const LEGACY_BRAND: &str = "MC|Brand";
pub const REGISTER: &str = "minecraft:register";
pub const LEGACY_REGISTER: &str = "REGISTER";
pub const UNREGISTER: &str = "minecraft:unregister";
pub const LEGACY_UNREGISTER: &str = "UNREGISTER";
pub const BUNGEE: &str = "bungeecord:main";
pub const LEGACY_BUNGEE: &str = "BungeeCord";

const fn modern(version: ProtocolVersion) -> bool {
    version.0 >= ProtocolVersion::V1_13.0
}

pub const fn brand_channel(version: ProtocolVersion) -> &'static str {
    if modern(version) { BRAND } else { LEGACY_BRAND }
}

pub const fn register_channel(version: ProtocolVersion) -> &'static str {
    if modern(version) {
        REGISTER
    } else {
        LEGACY_REGISTER
    }
}

pub const fn unregister_channel(version: ProtocolVersion) -> &'static str {
    if modern(version) {
        UNREGISTER
    } else {
        LEGACY_UNREGISTER
    }
}

pub const fn bungee_channel(version: ProtocolVersion) -> &'static str {
    if modern(version) {
        BUNGEE
    } else {
        LEGACY_BUNGEE
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMessage {
    pub channel: String,
    pub data: Vec<u8>,
}

impl PluginMessage {
    pub fn new(channel: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        Self {
            channel: channel.into(),
            data: data.into(),
        }
    }
}

pub fn brand_data(brand: &str, version: ProtocolVersion) -> HarnessResult<Vec<u8>> {
    if version.less_than(ProtocolVersion::V1_8) {
        return Ok(brand.as_bytes().to_vec());
    }
    let mut data = Vec::new();
    data.write_string(brand)?;
    Ok(data)
}

pub fn parse_brand(data: &[u8], version: ProtocolVersion) -> HarnessResult<String> {
    if version.less_than(ProtocolVersion::V1_8) {
        return Ok(String::from_utf8_lossy(data).into_owned());
    }
    let mut r = data;
    Ok(r.read_string()?)
}

pub fn register_data<S: AsRef<str>>(channels: &[S]) -> Vec<u8> {
    channels
        .iter()
        .map(AsRef::as_ref)
        .collect::<Vec<_>>()
        .join("\0")
        .into_bytes()
}

pub fn parse_register(data: &[u8]) -> Vec<String> {
    data.split(|b| *b == 0)
        .filter(|name| !name.is_empty())
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .collect()
}

fn legacy_body(data: &[u8], version: ProtocolVersion) -> HarnessResult<Vec<u8>> {
    if version.no_less_than(ProtocolVersion::V1_8) {
        return Ok(data.to_vec());
    }
    let len = i16::try_from(data.len())
        .map_err(|_| HarnessError::Unsupported("a 1.7 plugin message over 32767 bytes".into()))?;
    let mut body = Vec::with_capacity(data.len() + 2);
    body.write_i16_be(len)?;
    body.extend_from_slice(data);
    Ok(body)
}

fn strip_legacy(data: Vec<u8>, version: ProtocolVersion) -> HarnessResult<Vec<u8>> {
    if version.no_less_than(ProtocolVersion::V1_8) {
        return Ok(data);
    }
    let mut r = data.as_slice();
    let len = usize::try_from(r.read_i16_be()?)
        .map_err(|_| HarnessError::Unexpected("negative 1.7 plugin message length".into()))?;
    Ok(r.get(..len)
        .ok_or_else(|| HarnessError::Unexpected("short 1.7 plugin message".into()))?
        .to_vec())
}

pub fn to_backend(
    message: &PluginMessage,
    state: ConnectionState,
    version: ProtocolVersion,
) -> HarnessResult<PacketFrame> {
    let data = legacy_body(&message.data, version)?;
    match state {
        ConnectionState::Config => wire::encode(
            &SConfigPluginMessage {
                channel: message.channel.clone(),
                data,
            },
            version,
        ),
        _ => wire::encode(
            &SPluginMessage {
                channel: message.channel.clone(),
                data,
            },
            version,
        ),
    }
}

pub fn to_client(
    message: &PluginMessage,
    state: ConnectionState,
    version: ProtocolVersion,
) -> HarnessResult<PacketFrame> {
    let data = legacy_body(&message.data, version)?;
    match state {
        ConnectionState::Config => wire::encode(
            &CConfigPluginMessage {
                channel: message.channel.clone(),
                data,
            },
            version,
        ),
        _ => wire::encode(
            &CPluginMessage {
                channel: message.channel.clone(),
                data,
            },
            version,
        ),
    }
}

fn decode_with<P: Packet>(
    frame: &PacketFrame,
    version: ProtocolVersion,
    split: impl FnOnce(P) -> (String, Vec<u8>),
) -> HarnessResult<Option<PluginMessage>> {
    if !wire::is::<P>(frame, version) {
        return Ok(None);
    }
    let (channel, data) = split(wire::decode::<P>(frame, version)?);
    Ok(Some(PluginMessage {
        channel,
        data: strip_legacy(data, version)?,
    }))
}

pub fn serverbound(
    frame: &PacketFrame,
    state: ConnectionState,
    version: ProtocolVersion,
) -> HarnessResult<Option<PluginMessage>> {
    match state {
        ConnectionState::Config => {
            decode_with::<SConfigPluginMessage>(frame, version, |p| (p.channel, p.data))
        }
        _ => decode_with::<SPluginMessage>(frame, version, |p| (p.channel, p.data)),
    }
}

pub fn clientbound(
    frame: &PacketFrame,
    state: ConnectionState,
    version: ProtocolVersion,
) -> HarnessResult<Option<PluginMessage>> {
    match state {
        ConnectionState::Config => {
            decode_with::<CConfigPluginMessage>(frame, version, |p| (p.channel, p.data))
        }
        _ => decode_with::<CPluginMessage>(frame, version, |p| (p.channel, p.data)),
    }
}

pub fn client_information(
    frame: &PacketFrame,
    state: ConnectionState,
    version: ProtocolVersion,
) -> HarnessResult<Option<ClientInformation>> {
    match state {
        ConnectionState::Config if wire::is::<SConfigClientInformation>(frame, version) => Ok(
            Some(wire::decode::<SConfigClientInformation>(frame, version)?.information),
        ),
        ConnectionState::Play if wire::is::<SClientInformation>(frame, version) => Ok(Some(
            wire::decode::<SClientInformation>(frame, version)?.information,
        )),
        _ => Ok(None),
    }
}

pub fn information_frame(
    information: &ClientInformation,
    state: ConnectionState,
    version: ProtocolVersion,
) -> HarnessResult<PacketFrame> {
    match state {
        ConnectionState::Config => wire::encode(
            &SConfigClientInformation {
                information: information.clone(),
            },
            version,
        ),
        _ => wire::encode(
            &SClientInformation {
                information: information.clone(),
            },
            version,
        ),
    }
}

pub fn as_seen_by(
    information: &ClientInformation,
    version: ProtocolVersion,
) -> HarnessResult<ClientInformation> {
    let frame = information_frame(information, ConnectionState::Play, version)?;
    Ok(wire::decode::<SClientInformation>(&frame, version)?.information)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientHello {
    pub brand: Option<String>,
    pub information: Option<ClientInformation>,
    pub channels: Vec<String>,
}

impl ClientHello {
    pub fn sample() -> Self {
        Self {
            brand: Some("fabric".to_string()),
            information: Some(ClientInformation {
                locale: "fr_fr".to_string(),
                view_distance: 7,
                chat_mode: 1,
                chat_colors: false,
                difficulty: 2,
                displayed_skin_parts: 0x55,
                main_hand: 0,
                text_filtering: true,
                allow_server_listings: false,
                particle_status: 2,
            }),
            channels: vec!["infrarust:test".to_string(), "mod:sync".to_string()],
        }
    }

    pub fn frames(
        &self,
        state: ConnectionState,
        version: ProtocolVersion,
    ) -> HarnessResult<Vec<PacketFrame>> {
        let mut frames = Vec::new();
        let brand = self
            .brand
            .as_deref()
            .map(|brand| -> HarnessResult<PacketFrame> {
                let message =
                    PluginMessage::new(brand_channel(version), brand_data(brand, version)?);
                to_backend(&message, state, version)
            })
            .transpose()?;
        let information = self
            .information
            .as_ref()
            .map(|information| information_frame(information, state, version))
            .transpose()?;
        if state == ConnectionState::Config {
            frames.extend(brand);
            frames.extend(information);
        } else {
            frames.extend(information);
            frames.extend(brand);
        }
        if !self.channels.is_empty() {
            let message =
                PluginMessage::new(register_channel(version), register_data(&self.channels));
            frames.push(to_backend(&message, state, version)?);
        }
        Ok(frames)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObservedClientState {
    pub information: Option<ClientInformation>,
    pub brand: Option<String>,
    pub channels: Vec<String>,
    pub messages: Vec<PluginMessage>,
}

impl ObservedClientState {
    pub fn observe(
        &mut self,
        frame: &PacketFrame,
        state: ConnectionState,
        version: ProtocolVersion,
    ) -> HarnessResult<()> {
        if let Some(information) = client_information(frame, state, version)? {
            self.information = Some(information);
            return Ok(());
        }
        let Some(message) = serverbound(frame, state, version)? else {
            return Ok(());
        };
        if message.channel == brand_channel(version) {
            self.brand = Some(parse_brand(&message.data, version)?);
        } else if message.channel == register_channel(version) {
            for channel in parse_register(&message.data) {
                if !self.channels.contains(&channel) {
                    self.channels.push(channel);
                }
            }
        }
        self.messages.push(message);
        Ok(())
    }

    pub fn covers(&self, hello: &ClientHello) -> bool {
        (hello.information.is_none() || self.information.is_some())
            && (hello.brand.is_none() || self.brand.is_some())
            && hello.channels.iter().all(|c| self.channels.contains(c))
    }
}

pub async fn expect_client_state(
    conn: &mut BackendConn,
    hello: &ClientHello,
    timeout: Duration,
) -> HarnessResult<ObservedClientState> {
    let version = conn.version();
    let mut observed = ObservedClientState::default();
    for frame in conn.config_frames().to_vec() {
        observed.observe(&frame, ConnectionState::Config, version)?;
    }
    let deadline = Instant::now() + timeout;
    while !observed.covers(hello) {
        let left = deadline.saturating_duration_since(Instant::now());
        let frame = conn.recv_frame(left).await.map_err(|e| match e {
            HarnessError::Timeout { .. } => HarnessError::Unexpected(format!(
                "the backend never got the whole client state, it saw {observed:?}"
            )),
            other => other,
        })?;
        let state = conn.state();
        observed.observe(&frame, state, version)?;
    }
    Ok(observed)
}

pub async fn backend_message(
    conn: &mut BackendConn,
    channel: &str,
    timeout: Duration,
) -> HarnessResult<PluginMessage> {
    let version = conn.version();
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let frame = conn.recv_frame(left).await?;
        if let Some(message) = serverbound(&frame, conn.state(), version)?
            && message.channel == channel
        {
            return Ok(message);
        }
    }
}

pub async fn send_to_client(
    conn: &mut BackendConn,
    channel: &str,
    data: impl Into<Vec<u8>>,
) -> HarnessResult<()> {
    let frame = to_client(
        &PluginMessage::new(channel, data),
        conn.state(),
        conn.version(),
    )?;
    conn.send_frame(&frame).await
}

pub async fn send_to_backend(
    session: &ClientSession,
    channel: &str,
    data: impl Into<Vec<u8>>,
) -> HarnessResult<PacketFrame> {
    let frame = to_backend(
        &PluginMessage::new(channel, data),
        ConnectionState::Play,
        session.version(),
    )?;
    session.send_frame(&frame).await?;
    Ok(frame)
}

pub async fn client_message(
    session: &mut ClientSession,
    channel: &str,
    timeout: Duration,
) -> HarnessResult<PluginMessage> {
    let version = session.version();
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let frame = session.recv_frame(left).await?;
        if let Some(message) = clientbound(&frame, ConnectionState::Play, version)?
            && message.channel == channel
        {
            return Ok(message);
        }
    }
}

pub fn config_messages(
    session: &ClientSession,
    channel: &str,
) -> HarnessResult<Vec<PluginMessage>> {
    let version = session.version();
    let mut found = Vec::new();
    for frame in session.config_frames() {
        if let Some(message) = clientbound(frame, ConnectionState::Config, version)?
            && message.channel == channel
        {
            found.push(message);
        }
    }
    Ok(found)
}

pub fn bytes_of(message: &PluginMessage) -> Bytes {
    Bytes::from(message.data.clone())
}
