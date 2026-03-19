//! Login without backend -- completes the login/config phase for direct limbo entry.
//!
//! This module handles the case where the proxy needs to complete the
//! Minecraft login sequence without a real backend server. This is the
//! hardest part of limbo and will be implemented later when
//! initial-connection limbo is needed (e.g., auth gates before any backend).

use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use crate::error::CoreError;
use crate::session::client_bridge::ClientBridge;

/// Completes the login and configuration phases without a backend server.
///
/// This would send Login Success, enter the configuration phase, send
/// registry data and finish configuration, then transition to Play state --
/// all without a real backend.
///
/// # Current status
/// **Not yet implemented.** Returns an error. Initial-connection limbo
/// (where `SendToLimbo` is the very first action, before any backend
/// connection) requires this. For now, limbo is only supported after a
/// prior backend connection has already completed the login phase.
///
/// # Errors
/// Always returns [`CoreError::Other`] until implemented.
pub(crate) async fn _complete_login_without_backend(
    _client: &mut ClientBridge,
    _player_uuid: uuid::Uuid,
    _username: &str,
    _version: ProtocolVersion,
    _registry: &PacketRegistry,
    _compression_threshold: i32,
) -> Result<(), CoreError> {
    Err(CoreError::Other(
        "login without backend not yet implemented \
         -- initial SendToLimbo requires a prior backend connection"
            .to_string(),
    ))
}
