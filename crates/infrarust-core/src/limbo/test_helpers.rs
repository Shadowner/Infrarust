//! Shared test utilities for the limbo module.
//!
//! Provides common helpers used across limbo unit tests to avoid duplication.

#![cfg(test)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use bytes::Bytes;

use infrarust_api::types::GameProfile;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

/// Creates a test `GameProfile` with a nil UUID.
pub fn test_profile() -> GameProfile {
    GameProfile {
        uuid: uuid::Uuid::nil(),
        username: "LimboTester".to_string(),
        properties: vec![],
    }
}

/// Creates a default `PacketRegistry` for tests.
pub fn test_registry() -> PacketRegistry {
    infrarust_protocol::registry::build_default_registry()
}

/// Builds a `PacketFrame` from a typed packet using the registry.
///
/// Looks up the packet ID from the registry (serverbound, Play state)
/// and encodes the packet payload.
pub fn build_frame<P: Packet + 'static>(
    packet: &P,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> PacketFrame {
    let packet_id = registry
        .get_packet_id::<P>(ConnectionState::Play, Direction::Serverbound, version)
        .expect("packet ID should exist in registry");
    let mut payload = Vec::new();
    packet.encode(&mut payload, version).unwrap();
    PacketFrame {
        id: packet_id,
        payload: Bytes::from(payload),
    }
}
