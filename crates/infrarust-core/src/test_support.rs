use std::sync::OnceLock;

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::registry::{PacketRegistry, build_default_registry};
use infrarust_protocol::version::ProtocolVersion;

use crate::error::CoreError;
use crate::limbo::spawn::build_limbo_join_game;
use crate::player::packets::encode_packet;

fn registry() -> &'static PacketRegistry {
    static REGISTRY: OnceLock<PacketRegistry> = OnceLock::new();
    REGISTRY.get_or_init(build_default_registry)
}

pub fn join_game_frame(version: ProtocolVersion) -> Result<PacketFrame, CoreError> {
    let join = build_limbo_join_game(version)?;
    encode_packet(&join, version, registry())
}
