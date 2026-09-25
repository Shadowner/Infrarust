use std::fmt;
use std::sync::Arc;

use bytes::Bytes;

use crate::types::{ProtocolVersion, ServerId};

pub const MAX_CHANNEL_LENGTH: usize = 128;
pub const MAX_LEGACY_CHANNEL_LENGTH: usize = 20;
pub const MAX_TO_BACKEND_PAYLOAD: usize = 32_767;
pub const MAX_TO_CLIENT_PAYLOAD: usize = 1_048_576;

const BRAND: (&str, &str) = ("minecraft:brand", "MC|Brand");
const REGISTER: (&str, &str) = ("minecraft:register", "REGISTER");
const UNREGISTER: (&str, &str) = ("minecraft:unregister", "UNREGISTER");
const BUNGEECORD: (&str, &str) = ("bungeecord:main", "BungeeCord");
const BUILTIN_PAIRS: [(&str, &str); 4] = [BRAND, REGISTER, UNREGISTER, BUNGEECORD];

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Names {
    Modern(Arc<str>),
    Legacy(Arc<str>),
    Pair { modern: Arc<str>, legacy: Arc<str> },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChannelId(Names);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ChannelIdError {
    #[error(
        "`{0}` is not a namespaced channel: `namespace:name` in lowercase letters, digits, `_`, `-` and `.` (and `/` in the name), at most 128 characters"
    )]
    InvalidModern(String),
    #[error("`{0}` is not a legacy channel name: 1 to 20 characters, no NUL")]
    InvalidLegacy(String),
}

fn is_namespace_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.')
}

pub fn is_modern_channel(id: &str) -> bool {
    let Some((namespace, name)) = id.split_once(':') else {
        return false;
    };
    id.len() <= MAX_CHANNEL_LENGTH
        && !namespace.is_empty()
        && !name.is_empty()
        && namespace.chars().all(is_namespace_char)
        && name.chars().all(|c| is_namespace_char(c) || c == '/')
}

fn is_legacy_channel(name: &str) -> bool {
    let count = name.chars().count();
    (1..=MAX_LEGACY_CHANNEL_LENGTH).contains(&count) && !name.contains('\0')
}

fn builtin(pair: (&str, &str)) -> ChannelId {
    ChannelId(Names::Pair {
        modern: Arc::from(pair.0),
        legacy: Arc::from(pair.1),
    })
}

impl ChannelId {
    pub fn modern(id: &str) -> Result<Self, ChannelIdError> {
        if !is_modern_channel(id) {
            return Err(ChannelIdError::InvalidModern(id.to_string()));
        }
        Ok(BUILTIN_PAIRS
            .iter()
            .find(|(modern, _)| *modern == id)
            .map_or_else(|| Self(Names::Modern(Arc::from(id))), |pair| builtin(*pair)))
    }

    pub fn legacy(name: &str) -> Result<Self, ChannelIdError> {
        if !is_legacy_channel(name) {
            return Err(ChannelIdError::InvalidLegacy(name.to_string()));
        }
        Ok(BUILTIN_PAIRS
            .iter()
            .find(|(_, legacy)| *legacy == name)
            .map_or_else(
                || Self(Names::Legacy(Arc::from(name))),
                |pair| builtin(*pair),
            ))
    }

    pub fn pair(modern: &str, legacy: &str) -> Result<Self, ChannelIdError> {
        if !is_modern_channel(modern) {
            return Err(ChannelIdError::InvalidModern(modern.to_string()));
        }
        if !is_legacy_channel(legacy) {
            return Err(ChannelIdError::InvalidLegacy(legacy.to_string()));
        }
        Ok(Self(Names::Pair {
            modern: Arc::from(modern),
            legacy: Arc::from(legacy),
        }))
    }

    pub fn parse(raw: &str) -> Result<Self, ChannelIdError> {
        if is_modern_channel(raw) {
            Self::modern(raw)
        } else {
            Self::legacy(raw)
        }
    }

    pub fn brand() -> Self {
        builtin(BRAND)
    }

    pub fn register() -> Self {
        builtin(REGISTER)
    }

    pub fn unregister() -> Self {
        builtin(UNREGISTER)
    }

    pub fn bungeecord() -> Self {
        builtin(BUNGEECORD)
    }

    pub fn modern_id(&self) -> Option<&str> {
        match &self.0 {
            Names::Modern(modern) | Names::Pair { modern, .. } => Some(modern),
            Names::Legacy(_) => None,
        }
    }

    pub fn legacy_name(&self) -> Option<&str> {
        match &self.0 {
            Names::Legacy(legacy) | Names::Pair { legacy, .. } => Some(legacy),
            Names::Modern(_) => None,
        }
    }

    pub fn wire_name(&self, version: ProtocolVersion) -> &str {
        match &self.0 {
            Names::Modern(name) | Names::Legacy(name) => name,
            Names::Pair { modern, legacy } => {
                if version.at_least(ProtocolVersion::MINECRAFT_1_13) {
                    modern
                } else {
                    legacy
                }
            }
        }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.modern_id().into_iter().chain(self.legacy_name())
    }

    pub fn matches(&self, raw: &str) -> bool {
        self.names().any(|name| name == raw)
    }
}

impl fmt::Display for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Names::Modern(name) | Names::Legacy(name) => f.write_str(name),
            Names::Pair { modern, legacy } => write!(f, "{modern} ({legacy})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Endpoint {
    Client,
    Backend(ServerId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MessagePhase {
    Configuration,
    Play,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MessagingError {
    #[error("no player is connected to that server to carry the message")]
    NoCarrier,
    #[error("plugin message of {size} bytes is over the {max} bytes a backend accepts")]
    TooLarge { size: usize, max: usize },
}

pub mod private {
    pub trait Sealed {}
}

pub trait ChannelRegistrar: Send + Sync + private::Sealed {
    fn register(&self, channel: ChannelId);

    fn unregister(&self, channel: &ChannelId) -> bool;

    fn channels(&self) -> Vec<ChannelId>;
}

pub trait ServerMessenger: Send + Sync + private::Sealed {
    fn send_to_server(
        &self,
        server: &ServerId,
        channel: &ChannelId,
        data: Bytes,
    ) -> Result<usize, MessagingError>;
}

pub(crate) struct Inert;

impl private::Sealed for Inert {}

impl ChannelRegistrar for Inert {
    fn register(&self, _channel: ChannelId) {}

    fn unregister(&self, _channel: &ChannelId) -> bool {
        false
    }

    fn channels(&self) -> Vec<ChannelId> {
        Vec::new()
    }
}

impl ServerMessenger for Inert {
    fn send_to_server(
        &self,
        _server: &ServerId,
        _channel: &ChannelId,
        _data: Bytes,
    ) -> Result<usize, MessagingError> {
        Err(MessagingError::NoCarrier)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn modern_ids_are_validated() {
        assert!(ChannelId::modern("myplugin:main").is_ok());
        assert!(ChannelId::modern("my.plugin:sub/path-1").is_ok());
        for bad in [
            "noseparator",
            ":name",
            "ns:",
            "Upper:case",
            "ns:bad char",
            "a:b:c",
        ] {
            assert_eq!(
                ChannelId::modern(bad),
                Err(ChannelIdError::InvalidModern(bad.to_string())),
                "{bad}"
            );
        }
        let long = format!("a:{}", "b".repeat(MAX_CHANNEL_LENGTH));
        assert!(ChannelId::modern(&long).is_err());
    }

    #[test]
    fn legacy_names_are_validated() {
        assert!(ChannelId::legacy("MyChannel").is_ok());
        assert!(ChannelId::legacy("").is_err());
        assert!(ChannelId::legacy("twenty-one-characters").is_err());
        assert!(ChannelId::legacy("nul\0inside").is_err());
    }

    #[test]
    fn builtin_names_complete_their_pair() {
        assert_eq!(
            ChannelId::modern("minecraft:brand").unwrap(),
            ChannelId::brand()
        );
        assert_eq!(ChannelId::legacy("MC|Brand").unwrap(), ChannelId::brand());
        assert_eq!(
            ChannelId::legacy("BungeeCord").unwrap(),
            ChannelId::bungeecord()
        );
        assert_eq!(
            ChannelId::modern("minecraft:register").unwrap(),
            ChannelId::register()
        );
        assert_eq!(
            ChannelId::parse("UNREGISTER").unwrap(),
            ChannelId::unregister()
        );
    }

    #[test]
    fn the_wire_name_follows_the_protocol() {
        let bungee = ChannelId::bungeecord();
        assert_eq!(
            bungee.wire_name(ProtocolVersion::MINECRAFT_1_12),
            "BungeeCord"
        );
        assert_eq!(
            bungee.wire_name(ProtocolVersion::MINECRAFT_1_13),
            "bungeecord:main"
        );
        let modern = ChannelId::modern("myplugin:main").unwrap();
        assert_eq!(
            modern.wire_name(ProtocolVersion::MINECRAFT_1_8),
            "myplugin:main"
        );
        let legacy = ChannelId::legacy("MyChannel").unwrap();
        assert_eq!(
            legacy.wire_name(ProtocolVersion::MINECRAFT_1_21),
            "MyChannel"
        );
        let pair = ChannelId::pair("myplugin:main", "MyPlugin").unwrap();
        assert_eq!(pair.wire_name(ProtocolVersion::MINECRAFT_1_8), "MyPlugin");
        assert_eq!(
            pair.wire_name(ProtocolVersion::MINECRAFT_1_21),
            "myplugin:main"
        );
        assert!(pair.matches("MyPlugin"));
        assert!(pair.matches("myplugin:main"));
        assert!(!pair.matches("other:main"));
    }
}
