//! Forwarding configuration types.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::defaults;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardingMode {
    #[default]
    None,
    #[serde(alias = "legacy")]
    BungeeCord,
    BungeeGuard,
    #[serde(alias = "modern")]
    Velocity,
}

fn default_secret_file() -> PathBuf {
    PathBuf::from("forwarding.secret")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForwardingConfig {
    /// Default forwarding mode for all servers.
    #[serde(default)]
    pub mode: ForwardingMode,

    /// Path to the secret file (for Velocity or BungeeGuard).
    /// The file is created automatically if it doesn't exist.
    #[serde(default = "default_secret_file")]
    pub secret_file: PathBuf,

    #[serde(default, skip_serializing)]
    pub bungeecord_channel: Option<bool>,

    #[serde(default, skip_serializing)]
    pub channel_permissions: Option<BungeeCordChannelPermissions>,
}

impl ForwardingConfig {
    pub const fn has_moved_channel_keys(&self) -> bool {
        self.bungeecord_channel.is_some() || self.channel_permissions.is_some()
    }
}

impl Default for ForwardingConfig {
    fn default() -> Self {
        Self {
            mode: ForwardingMode::default(),
            secret_file: default_secret_file(),
            bungeecord_channel: None,
            channel_permissions: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct BungeeCordChannelPermissions {
    #[serde(default = "defaults::true_val", alias = "Connect")]
    pub connect: bool,
    #[serde(default, alias = "ConnectOther")]
    pub connect_other: bool,
    #[serde(default = "defaults::true_val", alias = "IP")]
    pub ip: bool,
    #[serde(default = "defaults::true_val", alias = "IPOther")]
    pub ip_other: bool,
    #[serde(default = "defaults::true_val", alias = "PlayerCount")]
    pub player_count: bool,
    #[serde(default = "defaults::true_val", alias = "PlayerList")]
    pub player_list: bool,
    #[serde(default = "defaults::true_val", alias = "GetServers")]
    pub get_servers: bool,
    #[serde(default = "defaults::true_val", alias = "GetServer")]
    pub get_server: bool,
    #[serde(default = "defaults::true_val", alias = "GetPlayerServer")]
    pub get_player_server: bool,
    #[serde(default = "defaults::true_val", alias = "Forward")]
    pub forward: bool,
    #[serde(default = "defaults::true_val", alias = "ForwardToPlayer")]
    pub forward_to_player: bool,
    #[serde(default = "defaults::true_val", alias = "UUID")]
    pub uuid: bool,
    #[serde(default = "defaults::true_val", alias = "UUIDOther")]
    pub uuid_other: bool,
    #[serde(default = "defaults::true_val", alias = "ServerIP")]
    pub server_ip: bool,
    #[serde(default, alias = "Message")]
    pub message: bool,
    #[serde(default, alias = "MessageRaw")]
    pub message_raw: bool,
    #[serde(default, alias = "KickPlayer")]
    pub kick_player: bool,
    #[serde(default, alias = "KickPlayerRaw")]
    pub kick_player_raw: bool,
}

impl Default for BungeeCordChannelPermissions {
    fn default() -> Self {
        Self {
            connect: true,
            connect_other: false,
            ip: true,
            ip_other: true,
            player_count: true,
            player_list: true,
            get_servers: true,
            get_server: true,
            get_player_server: true,
            forward: true,
            forward_to_player: true,
            uuid: true,
            uuid_other: true,
            server_ip: true,
            message: false,
            message_raw: false,
            kick_player: false,
            kick_player_raw: false,
        }
    }
}
