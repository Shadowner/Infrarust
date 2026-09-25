use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use bytes::Bytes;
use infrarust_api::messaging::{
    ChannelId, ChannelRegistrar, MAX_CHANNEL_LENGTH, MAX_TO_BACKEND_PAYLOAD, private,
};
use infrarust_protocol::codec::{McBufReadExt, McBufWriteExt, VarInt};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::config::{
    CConfigPluginMessage, SConfigClientInformation, SConfigPluginMessage,
};
use infrarust_protocol::packets::play::client_information::SClientInformation;
use infrarust_protocol::packets::play::plugin_message::{CPluginMessage, SPluginMessage};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

pub(crate) const MAX_KNOWN_CHANNELS: usize = 1024;
pub(crate) const MAX_BRAND_CHARS: usize = 128;

pub(crate) fn is_brand(raw: &str) -> bool {
    matches!(raw, "minecraft:brand" | "MC|Brand")
}

pub(crate) fn is_register(raw: &str) -> bool {
    matches!(raw, "minecraft:register" | "REGISTER")
}

pub(crate) fn is_unregister(raw: &str) -> bool {
    matches!(raw, "minecraft:unregister" | "UNREGISTER")
}

pub(crate) fn is_bungeecord(raw: &str) -> bool {
    matches!(raw, "bungeecord:main" | "BungeeCord")
}

pub(crate) fn reserved_for_backends(raw: &str) -> bool {
    is_bungeecord(raw) || raw.starts_with("velocity:")
}

pub(crate) fn parse_channels(data: &[u8]) -> Vec<String> {
    let mut channels: Vec<String> = Vec::new();
    for name in data.split(|b| *b == 0) {
        if channels.len() == MAX_KNOWN_CHANNELS {
            break;
        }
        if name.is_empty() || name.len() > MAX_CHANNEL_LENGTH {
            continue;
        }
        let Ok(name) = std::str::from_utf8(name) else {
            continue;
        };
        if !channels.iter().any(|known| known == name) {
            channels.push(name.to_string());
        }
    }
    channels
}

pub(crate) fn channel_payloads<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<Vec<u8>> {
    let mut payloads = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    for name in names {
        if !current.is_empty() && current.len() + 1 + name.len() > MAX_TO_BACKEND_PAYLOAD {
            payloads.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(0);
        }
        current.extend_from_slice(name.as_bytes());
    }
    if !current.is_empty() {
        payloads.push(current);
    }
    payloads
}

pub(crate) fn parse_brand(data: &[u8], version: ProtocolVersion) -> Option<String> {
    let brand = if version.no_less_than(ProtocolVersion::V1_8) {
        let mut r = data;
        r.read_string()
            .ok()
            .unwrap_or_else(|| String::from_utf8_lossy(data).into_owned())
    } else {
        String::from_utf8_lossy(data).into_owned()
    };
    let brand: String = brand.chars().take(MAX_BRAND_CHARS).collect();
    (!brand.is_empty()).then_some(brand)
}

pub(crate) fn brand_payload(brand: &str, version: ProtocolVersion) -> Vec<u8> {
    if version.less_than(ProtocolVersion::V1_8) {
        return brand.as_bytes().to_vec();
    }
    let mut data = Vec::with_capacity(brand.len() + 2);
    if data.write_string(brand).is_err() {
        data.clear();
    }
    data
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MessageIds {
    play_serverbound: Option<i32>,
    play_clientbound: Option<i32>,
    config_serverbound: Option<i32>,
    config_clientbound: Option<i32>,
    play_information: Option<i32>,
    config_information: Option<i32>,
}

impl MessageIds {
    pub(crate) fn resolve(registry: &PacketRegistry, version: ProtocolVersion) -> Self {
        Self {
            play_serverbound: registry.get_packet_id::<SPluginMessage>(version),
            play_clientbound: registry.get_packet_id::<CPluginMessage>(version),
            config_serverbound: registry.get_packet_id::<SConfigPluginMessage>(version),
            config_clientbound: registry.get_packet_id::<CConfigPluginMessage>(version),
            play_information: registry.get_packet_id::<SClientInformation>(version),
            config_information: registry.get_packet_id::<SConfigClientInformation>(version),
        }
    }

    pub(crate) const fn serverbound(&self, state: ConnectionState) -> Option<i32> {
        match state {
            ConnectionState::Play => self.play_serverbound,
            ConnectionState::Config => self.config_serverbound,
            _ => None,
        }
    }

    pub(crate) const fn clientbound(&self, state: ConnectionState) -> Option<i32> {
        match state {
            ConnectionState::Play => self.play_clientbound,
            ConnectionState::Config => self.config_clientbound,
            _ => None,
        }
    }

    pub(crate) const fn information(&self, state: ConnectionState) -> Option<i32> {
        match state {
            ConnectionState::Play => self.play_information,
            ConnectionState::Config => self.config_information,
            _ => None,
        }
    }

    pub(crate) fn is_serverbound(&self, frame: &PacketFrame, state: ConnectionState) -> bool {
        self.serverbound(state) == Some(frame.id)
    }

    pub(crate) fn is_clientbound(&self, frame: &PacketFrame, state: ConnectionState) -> bool {
        self.clientbound(state) == Some(frame.id)
    }

    pub(crate) fn is_information(&self, frame: &PacketFrame, state: ConnectionState) -> bool {
        self.information(state) == Some(frame.id)
    }
}

pub(crate) struct Peeked {
    pub(crate) channel: String,
    pub(crate) data: Bytes,
}

pub(crate) fn peek(frame: &PacketFrame, version: ProtocolVersion) -> Option<Peeked> {
    let mut r = frame.payload.as_ref();
    let len = usize::try_from(r.read_var_int().ok()?.0).ok()?;
    if len > r.len() || len > MAX_CHANNEL_LENGTH * 4 {
        return None;
    }
    let channel = std::str::from_utf8(&r[..len]).ok()?.to_string();
    let start = frame.payload.len() - r.len() + len;
    let mut data = frame.payload.slice(start..);
    if version.less_than(ProtocolVersion::V1_8) {
        let declared = usize::from(u16::from_be_bytes([*data.first()?, *data.get(1)?]));
        data = data.slice(2..(2 + declared).min(data.len()));
    }
    Some(Peeked { channel, data })
}

pub(crate) fn build(id: i32, channel: &str, data: &[u8], version: ProtocolVersion) -> PacketFrame {
    let mut payload = Vec::with_capacity(channel.len() + data.len() + 5);
    if payload.write_var_int(&VarInt(channel.len() as i32)).is_ok() {
        payload.extend_from_slice(channel.as_bytes());
    }
    if version.less_than(ProtocolVersion::V1_8) {
        let len = u16::try_from(data.len()).unwrap_or(u16::MAX);
        payload.extend_from_slice(&len.to_be_bytes());
        payload.extend_from_slice(&data[..usize::from(len)]);
    } else {
        payload.extend_from_slice(data);
    }
    PacketFrame::new(id, Bytes::from(payload))
}

struct Entry {
    channel: ChannelId,
    owners: Vec<Arc<str>>,
}

#[derive(Default)]
pub struct ChannelRegistry {
    entries: RwLock<HashMap<Box<str>, Entry>>,
    count: AtomicUsize,
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, owner: &Arc<str>, channel: &ChannelId) {
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        for name in channel.names() {
            let entry = entries.entry(Box::from(name)).or_insert_with(|| Entry {
                channel: channel.clone(),
                owners: Vec::new(),
            });
            if !entry.owners.contains(owner) {
                entry.owners.push(Arc::clone(owner));
            }
        }
        self.count.store(entries.len(), Ordering::Release);
    }

    pub fn unregister(&self, owner: &Arc<str>, channel: &ChannelId) -> bool {
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        let mut removed = false;
        for name in channel.names() {
            if let Some(entry) = entries.get_mut(name) {
                let before = entry.owners.len();
                entry.owners.retain(|o| o != owner);
                removed |= entry.owners.len() != before;
                if entry.owners.is_empty() {
                    entries.remove(name);
                }
            }
        }
        self.count.store(entries.len(), Ordering::Release);
        removed
    }

    pub fn remove_owner(&self, owner: &Arc<str>) {
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        entries.retain(|_, entry| {
            entry.owners.retain(|o| o != owner);
            !entry.owners.is_empty()
        });
        self.count.store(entries.len(), Ordering::Release);
    }

    pub fn lookup(&self, raw: &str) -> Option<ChannelId> {
        if self.count.load(Ordering::Acquire) == 0 {
            return None;
        }
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(raw)
            .map(|entry| entry.channel.clone())
    }

    pub fn owned_by(&self, owner: &Arc<str>) -> Vec<ChannelId> {
        let entries = self.entries.read().unwrap_or_else(PoisonError::into_inner);
        let mut channels: Vec<ChannelId> = Vec::new();
        for entry in entries.values() {
            if entry.owners.contains(owner) && !channels.contains(&entry.channel) {
                channels.push(entry.channel.clone());
            }
        }
        channels
    }

    pub fn wire_names(&self, version: ProtocolVersion) -> Vec<String> {
        let api_version = infrarust_api::types::ProtocolVersion::new(version.0);
        let entries = self.entries.read().unwrap_or_else(PoisonError::into_inner);
        let mut names: Vec<String> = Vec::new();
        for entry in entries.values() {
            let name = entry.channel.wire_name(api_version);
            if !names.iter().any(|known| known == name) {
                names.push(name.to_string());
            }
        }
        names.sort();
        names
    }
}

pub struct TrackingChannelRegistrar {
    registry: Arc<ChannelRegistry>,
    owner: Arc<str>,
}

impl TrackingChannelRegistrar {
    pub fn new(registry: Arc<ChannelRegistry>, owner: &str) -> Self {
        Self {
            registry,
            owner: Arc::from(owner),
        }
    }

    pub fn unregister_all(&self) {
        self.registry.remove_owner(&self.owner);
    }
}

impl private::Sealed for TrackingChannelRegistrar {}

impl ChannelRegistrar for TrackingChannelRegistrar {
    fn register(&self, channel: ChannelId) {
        self.registry.register(&self.owner, &channel);
    }

    fn unregister(&self, channel: &ChannelId) -> bool {
        self.registry.unregister(&self.owner, channel)
    }

    fn channels(&self) -> Vec<ChannelId> {
        self.registry.owned_by(&self.owner)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn channel_lists_are_capped_and_deduplicated() {
        let data = b"a:b\0\0c:d\0a:b\0\xFF\xFE\0e:f".to_vec();
        assert_eq!(parse_channels(&data), vec!["a:b", "c:d", "e:f"]);
        let long = "x".repeat(MAX_CHANNEL_LENGTH + 1);
        assert_eq!(
            parse_channels(format!("{long}\0ok:ok").as_bytes()),
            vec!["ok:ok"]
        );
        let many: Vec<String> = (0..MAX_KNOWN_CHANNELS + 10)
            .map(|i| format!("c:{i}"))
            .collect();
        assert_eq!(
            parse_channels(many.join("\0").as_bytes()).len(),
            MAX_KNOWN_CHANNELS
        );
    }

    #[test]
    fn channel_payloads_stay_under_the_serverbound_limit() {
        let names: Vec<String> = (0..MAX_KNOWN_CHANNELS)
            .map(|i| format!("plugin{i:04}:{}", "n".repeat(100)))
            .collect();
        let payloads = channel_payloads(names.iter().map(String::as_str));
        assert!(payloads.len() > 1);
        assert!(payloads.iter().all(|p| p.len() <= MAX_TO_BACKEND_PAYLOAD));
        let rejoined: Vec<String> = payloads.iter().flat_map(|p| parse_channels(p)).collect();
        assert_eq!(rejoined.len(), MAX_KNOWN_CHANNELS);
    }

    #[test]
    fn brands_parse_both_formats() {
        let modern = brand_payload("fabric", ProtocolVersion::V1_21);
        assert_eq!(modern, [6, b'f', b'a', b'b', b'r', b'i', b'c']);
        assert_eq!(
            parse_brand(&modern, ProtocolVersion::V1_21).as_deref(),
            Some("fabric")
        );
        assert_eq!(
            parse_brand(b"vanilla", ProtocolVersion::V1_7_2).as_deref(),
            Some("vanilla")
        );
        assert_eq!(parse_brand(b"", ProtocolVersion::V1_21), None);
        let long = brand_payload(&"b".repeat(500), ProtocolVersion::V1_21);
        assert_eq!(
            parse_brand(&long, ProtocolVersion::V1_21).map(|b| b.len()),
            Some(MAX_BRAND_CHARS)
        );
    }

    #[test]
    fn peek_reads_only_the_channel_and_keeps_the_data_zero_copy() {
        let frame = build(0x17, "mod:hello", b"payload", ProtocolVersion::V1_21);
        let peeked = peek(&frame, ProtocolVersion::V1_21).unwrap();
        assert_eq!(peeked.channel, "mod:hello");
        assert_eq!(peeked.data.as_ref(), b"payload");
        let legacy = build(0x17, "MC|Brand", b"vanilla", ProtocolVersion::V1_7_2);
        assert_eq!(&legacy.payload[9..11], &[0, 7]);
        let peeked = peek(&legacy, ProtocolVersion::V1_7_2).unwrap();
        assert_eq!(peeked.data.as_ref(), b"vanilla");
        assert!(
            peek(
                &PacketFrame::new(0, Bytes::from_static(&[5, b'a'])),
                ProtocolVersion::V1_21
            )
            .is_none()
        );
    }

    #[test]
    fn the_registry_tracks_owners() {
        let registry = ChannelRegistry::new();
        let a: Arc<str> = Arc::from("a");
        let b: Arc<str> = Arc::from("b");
        assert!(registry.lookup("BungeeCord").is_none());
        registry.register(&a, &ChannelId::bungeecord());
        registry.register(&b, &ChannelId::modern("b:main").unwrap());
        assert_eq!(registry.lookup("BungeeCord"), Some(ChannelId::bungeecord()));
        assert_eq!(
            registry.lookup("bungeecord:main"),
            Some(ChannelId::bungeecord())
        );
        assert_eq!(
            registry.wire_names(ProtocolVersion::V1_12_2),
            vec!["BungeeCord", "b:main"]
        );
        assert_eq!(registry.owned_by(&a), vec![ChannelId::bungeecord()]);
        registry.remove_owner(&a);
        assert!(registry.lookup("BungeeCord").is_none());
        assert!(registry.unregister(&b, &ChannelId::modern("b:main").unwrap()));
        assert!(!registry.unregister(&b, &ChannelId::modern("b:main").unwrap()));
        assert!(registry.lookup("b:main").is_none());
    }
}
