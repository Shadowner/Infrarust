//! Legacy Minecraft protocol support (Beta 1.8 – MC 1.6).
//!
//! The legacy protocol is completely outside the `VarInt` framing used by
//! modern (1.7+) Minecraft. This module provides detection, parsing, and
//! response construction for legacy ping and login connections.

pub mod ping;

pub use ping::{LegacyPingRequest, LegacyPingResponse, LegacyPingVariant, parse_legacy_ping};

/// Type of connection detected by the first byte of the TCP stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDetection {
    /// `0xFE` — Legacy ping (Beta 1.8 to MC 1.6, also supported by modern servers).
    LegacyPing,
    /// `0x02` — Legacy login (pre-Netty).
    LegacyLogin,
    /// Other — Modern protocol (1.7+), first byte is the start of a `VarInt` length.
    Modern,
}

/// Detects the type of connection by the first byte of the TCP stream.
///
/// Called by the transport layer before deciding which decoder to use
/// (`PacketDecoder` for modern, legacy handler for legacy).
pub const fn detect(first_byte: u8) -> LegacyDetection {
    match first_byte {
        0xFE => LegacyDetection::LegacyPing,
        0x02 => LegacyDetection::LegacyLogin,
        _ => LegacyDetection::Modern,
    }
}
