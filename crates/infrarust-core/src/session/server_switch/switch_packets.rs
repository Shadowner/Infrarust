//! Version-branched packet sending for server switch.
//!
//! Handles the "Respawn trick" — sending the right combination of JoinGame
//! and Respawn packets to the client depending on the protocol version.

use infrarust_protocol::Packet;
use infrarust_protocol::error::ProtocolError;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::play::dimension::{
    DimensionInfo, extract_dimension_from_join_game,
};
use infrarust_protocol::packets::play::respawn::CRespawn;
use infrarust_protocol::packets::play::respawn_switch;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::error::CoreError;
use crate::session::client_bridge::ClientBridge;

/// Sends the server switch packets to the client.
///
/// Expects the `join_game_frame` to already be read from the new backend.
/// Handles the version-branched trick:
/// - Pre-1.16: JoinGame + Respawn(temp_dim) + Respawn(real_dim)
/// - 1.16-1.20.1: JoinGame + Respawn(real_dim)
/// - 1.20.2+: JoinGame only (config phase already handled)
pub async fn send_switch_packets(
    client: &mut ClientBridge,
    join_game_frame: &PacketFrame,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    // Forward JoinGame opaque (always)
    client.write_frame(join_game_frame).await?;

    if version.no_less_than(ProtocolVersion::V1_20_2) {
        // 1.20.2+: config phase already done, JoinGame is enough
        return Ok(());
    }

    // Extract dimension from JoinGame for the Respawn trick
    let join_game = infrarust_protocol::packets::play::join_game::CJoinGame::decode(
        &mut join_game_frame.payload.as_ref(),
        version,
    )?;

    let raw_payload = join_game.raw_payload.as_deref().ok_or_else(|| {
        CoreError::Protocol(ProtocolError::invalid(
            "JoinGame pre-1.20.2 should have raw_payload",
        ))
    })?;

    let dimension = extract_dimension_from_join_game(raw_payload, version)?;

    if version.less_than(ProtocolVersion::V1_16) {
        // Pre-1.16: double Respawn trick
        // First: send Respawn with a DIFFERENT dimension to force world unload
        let temp_dim = temp_dimension(&dimension);
        send_respawn(client, &temp_dim, version, registry).await?;
        // Second: send Respawn with the REAL dimension
        send_respawn(client, &dimension, version, registry).await?;
    } else {
        // 1.16-1.20.1: single Respawn (JoinGame already triggers world reset)
        send_respawn(client, &dimension, version, registry).await?;
    }

    Ok(())
}

/// Sends a constructed Respawn packet to the client.
async fn send_respawn(
    client: &mut ClientBridge,
    dimension: &DimensionInfo,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let respawn = respawn_switch::for_switch(dimension, version);
    let packet_id = registry
        .get_packet_id::<CRespawn>(ConnectionState::Play, Direction::Clientbound, version)
        .ok_or_else(|| CoreError::Protocol(ProtocolError::invalid("no Respawn packet ID")))?;

    let mut payload = Vec::new();
    respawn.encode(&mut payload, version)?;

    let frame = PacketFrame {
        id: packet_id,
        payload: payload.into(),
    };
    client.write_frame(&frame).await?;
    Ok(())
}

/// Returns a temporary dimension different from the given one (pre-1.16 double-respawn trick).
fn temp_dimension(dim: &DimensionInfo) -> DimensionInfo {
    match dim {
        DimensionInfo::Legacy(id) => {
            if *id == 0 {
                DimensionInfo::Legacy(-1)
            } else {
                DimensionInfo::Legacy(0)
            }
        }
        DimensionInfo::Named(name) => {
            if name == "minecraft:overworld" {
                DimensionInfo::Named("minecraft:the_nether".to_string())
            } else {
                DimensionInfo::Named("minecraft:overworld".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn test_temp_dimension_overworld() {
        let dim = DimensionInfo::Legacy(0);
        assert_eq!(temp_dimension(&dim), DimensionInfo::Legacy(-1));
    }

    #[test]
    fn test_temp_dimension_nether() {
        let dim = DimensionInfo::Legacy(-1);
        assert_eq!(temp_dimension(&dim), DimensionInfo::Legacy(0));
    }

    #[test]
    fn test_temp_dimension_end() {
        let dim = DimensionInfo::Legacy(1);
        assert_eq!(temp_dimension(&dim), DimensionInfo::Legacy(0));
    }

    #[test]
    fn test_temp_dimension_named_overworld() {
        let dim = DimensionInfo::Named("minecraft:overworld".to_string());
        assert_eq!(
            temp_dimension(&dim),
            DimensionInfo::Named("minecraft:the_nether".to_string())
        );
    }

    #[test]
    fn test_temp_dimension_named_nether() {
        let dim = DimensionInfo::Named("minecraft:the_nether".to_string());
        assert_eq!(
            temp_dimension(&dim),
            DimensionInfo::Named("minecraft:overworld".to_string())
        );
    }
}
