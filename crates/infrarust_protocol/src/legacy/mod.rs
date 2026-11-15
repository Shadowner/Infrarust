pub mod handshake;
pub mod ping;

pub use handshake::{LegacyHandshakeRequest, build_legacy_kick, parse_legacy_handshake};
pub use ping::{
    LegacyPingRequest, LegacyPingResponse, LegacyPingVariant, MAX_PING_CHANNEL_CHARS,
    MAX_PING_DATA_BYTES, parse_legacy_ping,
};

pub const MAX_USERNAME_CHARS: usize = 16;
pub const MAX_HOSTNAME_CHARS: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDetection {
    LegacyPing,
    LegacyLogin,
    Modern,
}

pub const fn detect(first_byte: u8) -> LegacyDetection {
    match first_byte {
        0xFE => LegacyDetection::LegacyPing,
        0x02 => LegacyDetection::LegacyLogin,
        _ => LegacyDetection::Modern,
    }
}
