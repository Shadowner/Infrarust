//! Codec-level filter types.
//!
//! Codec filters operate on framed Minecraft packets (`RawPacket`).
//! They are synchronous (no async) and run inline in the proxy loop
//! for every packet — they must be fast (< 1 us).

use std::net::{IpAddr, SocketAddr};

use crate::event::ConnectionState;
use crate::types::{ProtocolVersion, RawPacket};

use super::metadata::FilterMetadata;

/// Global factory registered once. Creates per-connection instances.
///
/// `Send + Sync` because it is shared across all connections.
/// Called **twice** per session: once for client-side, once for server-side.
pub trait CodecFilterFactory: Send + Sync {
    /// Returns the factory's metadata for ordering.
    fn metadata(&self) -> FilterMetadata;

    /// Creates a filter instance for a new connection/side.
    fn create(&self, ctx: &CodecSessionInit) -> Box<dyn CodecFilterInstance>;
}

/// Per-connection filter instance with mutable state.
///
/// `Send` but NOT `Sync` — each instance lives in a single tokio task.
/// All methods are **synchronous** (no async) since they run on the
/// packet hot path.
pub trait CodecFilterInstance: Send {
    /// Filters a single packet.
    ///
    /// May modify the packet in place, inject additional packets via
    /// `output`, or return a verdict to drop/replace the packet.
    fn filter(
        &mut self,
        ctx: &CodecContext,
        packet: &mut RawPacket,
        output: &mut FrameOutput,
    ) -> CodecVerdict;

    /// Called when the protocol state changes (e.g. Login -> Config -> Play).
    fn on_state_change(&mut self, _new_state: ConnectionState) {}

    /// Called when compression is activated or the threshold changes.
    fn on_compression_change(&mut self, _threshold: i32) {}

    /// Called when encryption is enabled.
    fn on_encryption_enabled(&mut self) {}

    /// Cleanup when the connection terminates.
    fn on_close(&mut self) {}
}

/// Information passed to the factory when creating a filter instance.
#[derive(Debug, Clone)]
pub struct CodecSessionInit {
    /// The client's protocol version.
    pub client_version: ProtocolVersion,
    /// Unique connection identifier.
    pub connection_id: u64,
    /// The client's remote address.
    pub remote_addr: SocketAddr,
    /// The real client IP (if behind a proxy).
    pub real_ip: Option<IpAddr>,
    /// Which side of the connection this instance filters.
    pub side: ConnectionSide,
}

/// Context passed to each filter invocation.
#[derive(Debug, Clone)]
pub struct CodecContext {
    /// The client's protocol version.
    pub client_version: ProtocolVersion,
    /// The backend server's protocol version (if known).
    pub server_version: Option<ProtocolVersion>,
    /// Current protocol state.
    pub state: ConnectionState,
    /// Unique connection identifier.
    pub connection_id: u64,
    /// Which side of the connection this filter is on.
    pub side: ConnectionSide,
    /// Player info (available after login).
    pub player_info: Option<PlayerInfo>,
    /// `true` if the proxy consumes this packet (KeepAlive, SetCompression, etc.).
    pub is_proxy_consumed: bool,
}

/// Information about the connected player.
#[derive(Debug, Clone)]
pub struct PlayerInfo {
    /// The player's username.
    pub username: String,
    /// The player's UUID (if authenticated).
    pub uuid: Option<uuid::Uuid>,
}

/// Which side of the proxy connection a filter instance handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionSide {
    /// Packets from the client to the proxy.
    ClientSide,
    /// Packets from the backend server to the proxy.
    ServerSide,
}

/// Verdict returned by a codec filter.
#[derive(Debug)]
pub enum CodecVerdict {
    /// Let the packet through (possibly modified in place).
    Pass,
    /// Drop the packet entirely.
    Drop,
    /// Replace the original packet with the injected frames in `FrameOutput`.
    Replace,
    /// An error occurred in the filter.
    Error(CodecFilterError),
}

/// Errors that can occur within a codec filter.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CodecFilterError {
    /// Packet translation between versions failed.
    #[error("packet translation failed: {0}")]
    TranslationFailed(String),
    /// The packet payload is malformed.
    #[error("malformed packet payload")]
    MalformedPayload,
    /// The protocol version is not supported by this filter.
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(i32),
    /// An internal error within the filter.
    #[error("internal filter error: {0}")]
    Internal(String),
}

/// Allows a filter to inject additional packets before/after the current one.
#[derive(Debug, Default)]
pub struct FrameOutput {
    frames_before: Vec<RawPacket>,
    frames_after: Vec<RawPacket>,
}

impl FrameOutput {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Injects a packet to be sent BEFORE the current packet.
    pub fn inject_before(&mut self, packet: RawPacket) {
        self.frames_before.push(packet);
    }

    /// Injects a packet to be sent AFTER the current packet.
    pub fn inject_after(&mut self, packet: RawPacket) {
        self.frames_after.push(packet);
    }

    /// Returns `true` if any packets have been injected.
    #[must_use]
    pub fn has_injections(&self) -> bool {
        !self.frames_before.is_empty() || !self.frames_after.is_empty()
    }

    /// Takes and returns all "before" injected packets.
    pub fn take_before(&mut self) -> Vec<RawPacket> {
        std::mem::take(&mut self.frames_before)
    }

    /// Takes and returns all "after" injected packets.
    pub fn take_after(&mut self) -> Vec<RawPacket> {
        std::mem::take(&mut self.frames_after)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn frame_output_inject_before_and_after() {
        let mut output = FrameOutput::new();
        assert!(!output.has_injections());

        output.inject_before(RawPacket::new(0x01, bytes::Bytes::from_static(b"a")));
        output.inject_after(RawPacket::new(0x02, bytes::Bytes::from_static(b"b")));
        assert!(output.has_injections());

        let before = output.take_before();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].packet_id, 0x01);

        let after = output.take_after();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].packet_id, 0x02);
    }

    #[test]
    fn frame_output_take_drains_vecs() {
        let mut output = FrameOutput::new();
        output.inject_before(RawPacket::new(0x01, bytes::Bytes::new()));
        let _ = output.take_before();
        assert!(!output.has_injections());
    }
}
