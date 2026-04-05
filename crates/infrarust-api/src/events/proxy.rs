//! Proxy-level events.

use crate::event::Event;
use crate::services::server_manager::ServerState;
use crate::types::{Component, ProtocolVersion, ServerId};

/// Fired when the proxy receives a server list ping request.
///
/// Listeners can modify the response to customize the MOTD,
/// player count, protocol version, and favicon.
pub struct ProxyPingEvent {
    /// The remote address of the pinging client.
    pub remote_addr: std::net::SocketAddr,
    /// The mutable ping response that will be sent back.
    pub response: PingResponse,
}

impl ProxyPingEvent {
    pub const fn response_mut(&mut self) -> &mut PingResponse {
        &mut self.response
    }
}

/// The server list ping response data.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PingResponse {
    /// The MOTD description shown in the server list.
    pub description: Component,
    /// Maximum player count shown in the server list.
    pub max_players: i32,
    /// Current online player count shown in the server list.
    pub online_players: i32,
    /// The protocol version to report.
    pub protocol_version: ProtocolVersion,
    /// The version name string (e.g. "Infrarust 2.0").
    pub version_name: String,
    /// Base64-encoded 64x64 PNG favicon, if any.
    pub favicon: Option<String>,
}

impl PingResponse {
    pub fn new(
        description: Component,
        max_players: i32,
        online_players: i32,
        protocol_version: ProtocolVersion,
        version_name: String,
        favicon: Option<String>,
    ) -> Self {
        Self {
            description,
            max_players,
            online_players,
            protocol_version,
            version_name,
            favicon,
        }
    }
}

impl Event for ProxyPingEvent {}

/// Fired when the proxy has finished initializing.
///
/// Plugins can use this to perform post-startup setup that depends on
/// all other plugins being loaded.
pub struct ProxyInitializeEvent;

impl Event for ProxyInitializeEvent {}

/// Fired when the proxy is shutting down.
///
/// Plugins should use this (or [`Plugin::on_disable`](crate::plugin::Plugin::on_disable))
/// to clean up resources.
pub struct ProxyShutdownEvent;

impl Event for ProxyShutdownEvent {}

/// Fired when the proxy configuration is hot-reloaded.
///
/// Plugins can re-read their configuration in response.
pub struct ConfigReloadEvent;

impl Event for ConfigReloadEvent {}

/// Fired when a backend server changes state.
pub struct ServerStateChangeEvent {
    /// The server whose state changed.
    pub server: ServerId,
    pub old_state: ServerState,
    pub new_state: ServerState,
}

impl Event for ServerStateChangeEvent {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn ping_response_mutation() {
        let mut event = ProxyPingEvent {
            remote_addr: "127.0.0.1:12345".parse().unwrap(),
            response: PingResponse::new(
                Component::text("Hello"),
                100,
                42,
                ProtocolVersion::MINECRAFT_1_21,
                "Infrarust 2.0".into(),
                None,
            ),
        };

        event.response_mut().online_players = 99;
        event.response_mut().description = Component::text("Updated MOTD").color("gold");
        assert_eq!(event.response.online_players, 99);
    }
}
