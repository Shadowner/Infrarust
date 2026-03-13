use std::sync::Arc;

use bytes::BytesMut;
use infrarust_config::ServerConfig;
use infrarust_protocol::ProtocolVersion;
use uuid::Uuid;

/// Intent extracted from the Minecraft handshake packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionIntent {
    Status,
    Login,
    Transfer,
}

/// Data extracted by the handshake parser middleware.
#[derive(Debug, Clone)]
pub struct HandshakeData {
    /// Cleaned domain (FML markers stripped).
    pub domain: String,
    /// Port from handshake.
    pub port: u16,
    /// Protocol version from handshake.
    pub protocol_version: ProtocolVersion,
    /// Client intent (status, login, transfer).
    pub intent: ConnectionIntent,
    /// Raw packet bytes for forwarding to backend in passthrough mode.
    pub raw_packets: Vec<BytesMut>,
}

/// Data resolved by the domain router middleware.
#[derive(Debug, Clone)]
pub struct RoutingData {
    /// Matched server configuration.
    pub server_config: Arc<ServerConfig>,
    /// Config identifier (filename stem).
    pub config_id: String,
}

/// Data extracted by the login start parser middleware.
#[derive(Debug, Clone)]
pub struct LoginData {
    /// Player username from LoginStart packet.
    pub username: String,
    /// Player UUID (present in 1.20.2+).
    pub player_uuid: Option<Uuid>,
}

/// Marker type inserted into extensions when a legacy ping is detected (first byte 0xFE).
#[derive(Debug, Clone, Copy)]
pub struct LegacyDetected;
