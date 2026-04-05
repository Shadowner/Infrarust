//! Forwarding configuration types.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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

fn default_true() -> bool {
    true
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

    #[serde(default = "default_true")]
    pub bungeecord_channel: bool,

    #[serde(default)]
    pub channel_permissions: BungeeCordChannelPermissions,
}

impl Default for ForwardingConfig {
    fn default() -> Self {
        Self {
            mode: ForwardingMode::default(),
            secret_file: default_secret_file(),
            bungeecord_channel: true,
            channel_permissions: BungeeCordChannelPermissions::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct BungeeCordChannelPermissions {
    #[serde(default = "default_true", alias = "Connect")]
    pub connect: bool,
    #[serde(default, alias = "ConnectOther")]
    pub connect_other: bool,
    #[serde(default = "default_true", alias = "IP")]
    pub ip: bool,
    #[serde(default = "default_true", alias = "IPOther")]
    pub ip_other: bool,
    #[serde(default = "default_true", alias = "PlayerCount")]
    pub player_count: bool,
    #[serde(default = "default_true", alias = "PlayerList")]
    pub player_list: bool,
    #[serde(default = "default_true", alias = "GetServers")]
    pub get_servers: bool,
    #[serde(default = "default_true", alias = "GetServer")]
    pub get_server: bool,
    #[serde(default = "default_true", alias = "GetPlayerServer")]
    pub get_player_server: bool,
    #[serde(default = "default_true", alias = "Forward")]
    pub forward: bool,
    #[serde(default = "default_true", alias = "ForwardToPlayer")]
    pub forward_to_player: bool,
    #[serde(default = "default_true", alias = "UUID")]
    pub uuid: bool,
    #[serde(default = "default_true", alias = "UUIDOther")]
    pub uuid_other: bool,
    #[serde(default = "default_true", alias = "ServerIP")]
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
