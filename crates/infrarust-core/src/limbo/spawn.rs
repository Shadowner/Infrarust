//! Spawn sequence -- version-branched packet sequence for entering limbo.
//!
//! Sends the minimal set of packets needed for the client to enter the
//! limbo world (an empty flat void). The exact sequence depends on the
//! protocol version and whether this is a fresh join or a switch into limbo
//! from an existing backend connection.

use bytes::Bytes;
use infrarust_protocol::codec::{McBufWriteExt, VarInt};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::play::center_chunk::CSetCenterChunk;
use infrarust_protocol::packets::play::chunk_batch::{CChunkBatchFinished, CChunkBatchStart};
use infrarust_protocol::packets::play::dimension::DimensionInfo;
use infrarust_protocol::packets::play::game_event::{CGameEvent, START_WAITING_CHUNKS};
use infrarust_protocol::packets::play::join_game::CJoinGame;
use infrarust_protocol::packets::play::player_position::CSynchronizePlayerPosition;
use infrarust_protocol::packets::play::respawn::CRespawn;
use infrarust_protocol::packets::play::respawn_switch;
use infrarust_protocol::packets::play::spawn_position::CSetDefaultSpawnPosition;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use infrarust_protocol::chunk::build_chunk_data_frame;
use crate::error::CoreError;
use crate::player::packets::encode_packet;
use crate::session::client_bridge::ClientBridge;

const LIMBO_DIMENSION_NAME: &str = "minecraft:the_end";
const LIMBO_DIMENSION_ID: i32 = 2;
const LIMBO_NUM_SECTIONS: usize = 16;
const LIMBO_DIM: DimensionInfo = DimensionInfo::Legacy(1);

pub(crate) async fn send_spawn_sequence(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
    needs_join_game: bool,
) -> Result<(), CoreError> {
    let is_modern = version.no_less_than(ProtocolVersion::V1_20_2);
    let is_pre_1_16 = version.less_than(ProtocolVersion::V1_16);

    if is_modern && needs_join_game {
        send_modern_with_join(client, version, registry).await?;
    } else if is_modern {
        send_modern_switch(client, version, registry).await?;
    } else if is_pre_1_16 && needs_join_game {
        send_pre_1_16_with_join(client, version, registry).await?;
    } else if is_pre_1_16 {
        send_pre_1_16_switch(client, version, registry).await?;
    } else if needs_join_game {
        send_legacy_with_join(client, version, registry).await?;
    } else {
        send_legacy_switch(client, version, registry).await?;
    }

    // Inventory clear uses hardcoded packet IDs that are only valid for 1.16+.
    // Pre-1.16 has a different wire format; skip it (inventory starts empty on
    // fresh JoinGame, and adventure mode prevents interaction anyway).
    if !is_pre_1_16 {
        send_clear_inventory(client, version).await?;
    }

    Ok(())
}

async fn send_modern_with_join(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    send_join_game(client, version, registry).await?;
    send_spawn_position(client, version, registry).await?;
    send_player_position(client, version, registry).await?;
    send_modern_chunk_setup(client, version, registry).await
}

async fn send_modern_switch(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    send_modern_chunk_setup(client, version, registry).await?;
    send_player_position(client, version, registry).await
}

/// Pre-1.16 fresh join: JoinGame + double-Respawn trick + PlayerPosition + Chunk.
///
/// The double-Respawn forces the client to reload the world (dimension change
/// to Overworld then back to The End). Without it, the client stays stuck on
/// "Loading terrain". The chunk at (0,0) is also sent so the client has at
/// least one column loaded.
async fn send_pre_1_16_with_join(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    send_join_game(client, version, registry).await?;
    send_limbo_respawn(client, &DimensionInfo::Legacy(0), version, registry).await?;
    send_limbo_respawn(client, &LIMBO_DIM, version, registry).await?;
    send_player_position(client, version, registry).await?;
    send_chunk(client, version, registry).await
}

async fn send_pre_1_16_switch(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    send_limbo_respawn(client, &DimensionInfo::Legacy(0), version, registry).await?;
    send_limbo_respawn(client, &LIMBO_DIM, version, registry).await?;
    send_player_position(client, version, registry).await?;
    send_chunk(client, version, registry).await
}

async fn send_legacy_with_join(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    send_join_game(client, version, registry).await?;
    send_player_position(client, version, registry).await?;
    send_chunk(client, version, registry).await
}

async fn send_legacy_switch(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    send_player_position(client, version, registry).await?;
    send_chunk(client, version, registry).await
}

async fn send_join_game(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let join = build_limbo_join_game(version)?;
    let frame = encode_packet(&join, version, registry)?;
    client.write_frame(&frame).await
}

async fn send_spawn_position(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let spawn = CSetDefaultSpawnPosition::at_in(LIMBO_DIMENSION_NAME, 0, 64, 0, 0.0);
    let frame = encode_packet(&spawn, version, registry)?;
    client.write_frame(&frame).await
}

async fn send_player_position(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let pos = limbo_player_position(version);
    let frame = encode_packet(&pos, version, registry)?;
    client.write_frame(&frame).await
}

async fn send_chunk(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    _registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let frame = build_chunk_data_frame(0, 0, LIMBO_NUM_SECTIONS, version)?;
    client.write_frame(&frame).await
}

async fn send_limbo_respawn(
    client: &mut ClientBridge,
    dimension: &DimensionInfo,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let respawn = respawn_switch::for_switch(dimension, version);
    let packet_id = registry
        .get_packet_id::<CRespawn>(ConnectionState::Play, Direction::Clientbound, version)
        .ok_or_else(|| CoreError::Other("no Respawn packet ID".to_string()))?;

    let mut payload = Vec::new();
    respawn.encode(&mut payload, version)?;

    let frame = PacketFrame {
        id: packet_id,
        payload: payload.into(),
    };
    client.write_frame(&frame).await
}

async fn send_modern_chunk_setup(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let center = CSetCenterChunk { chunk_x: 0, chunk_z: 0 };
    let frame = encode_packet(&center, version, registry)?;
    client.write_frame(&frame).await?;

    let event = CGameEvent { event: START_WAITING_CHUNKS, value: 0.0 };
    let frame = encode_packet(&event, version, registry)?;
    client.write_frame(&frame).await?;

    let frame = encode_packet(&CChunkBatchStart, version, registry)?;
    client.write_frame(&frame).await?;

    send_chunk(client, version, registry).await?;

    let batch_done = CChunkBatchFinished { batch_size: 1 };
    let frame = encode_packet(&batch_done, version, registry)?;
    client.write_frame(&frame).await
}

fn limbo_player_position(version: ProtocolVersion) -> CSynchronizePlayerPosition {
    // Pre-1.16: y=400 (above old build limit 256, no chunks needed).
    // 1.16+: y=64 (normal; chunk data is sent).
    let y = if version.less_than(ProtocolVersion::V1_16) {
        400.0
    } else {
        64.0
    };

    CSynchronizePlayerPosition {
        x: 0.0,
        y,
        z: 0.0,
        delta_x: 0.0,
        delta_y: 0.0,
        delta_z: 0.0,
        yaw: 0.0,
        pitch: 0.0,
        flags: 0,
        teleport_id: 0,
    }
}

fn build_limbo_join_game(version: ProtocolVersion) -> Result<CJoinGame, CoreError> {
    if version.less_than(ProtocolVersion::V1_16) {
        let raw_payload = build_pre_1_16_join_game_payload(version)?;
        return Ok(CJoinGame {
            entity_id: 0,
            raw_payload: Some(raw_payload),
            ..Default::default()
        });
    }

    if version.less_than(ProtocolVersion::V1_20_2) {
        return Err(CoreError::Other(
            "limbo JoinGame construction for 1.16\u{2013}1.20.1 is not yet implemented"
                .to_string(),
        ));
    }

    Ok(CJoinGame {
        entity_id: 0,
        is_hardcore: false,
        gamemode: 2, // adventure
        previous_gamemode: -1,
        max_players: 1,
        view_distance: 2,
        simulation_distance: 2,
        reduced_debug_info: false,
        enable_respawn_screen: true,
        do_limited_crafting: false,
        level_names: vec![LIMBO_DIMENSION_NAME.to_string()],
        level_name: LIMBO_DIMENSION_NAME.to_string(),
        hashed_seed: 0,
        is_debug: false,
        is_flat: false,
        dimension: LIMBO_DIMENSION_ID,
        portal_cooldown: 0,
        sea_level: 0, // End has no sea
        enforces_secure_chat: false,
        death_dimension: None,
        death_position: None,
        raw_payload: None,
    })
}

/// Builds the JoinGame raw payload (everything after `entity_id`) for pre-1.16.
///
/// - 1.7: gamemode(u8) dimension(i8) difficulty(u8) max_players(u8) level_type(String)
/// - 1.8: + reduced_debug_info(bool)
/// - 1.9–1.13: dimension becomes i32
/// - 1.14: no difficulty, max_players is VarInt, + view_distance(VarInt)
/// - 1.15: + hashed_seed(i64), max_players back to u8, + enable_respawn_screen(bool)
fn build_pre_1_16_join_game_payload(
    version: ProtocolVersion,
) -> Result<Vec<u8>, CoreError> {
    let mut buf = Vec::with_capacity(32);

    // gamemode always u8, adventure mode (2)
    buf.write_u8(2)?;

    // dimension — The End (1)
    if version.less_than(ProtocolVersion::V1_9) {
        // 1.7–1.8: dimension as i8
        buf.write_i8(1)?;
    } else {
        // 1.9+: dimension as i32
        buf.write_i32_be(1)?;
    }

    // difficulty removed in 1.14
    if version.less_than(ProtocolVersion::V1_14) {
        buf.write_u8(0)?; // peaceful
    }

    // hashed_seed added in 1.15 (between difficulty removal and max_players)
    if version.no_less_than(ProtocolVersion::V1_15) {
        buf.write_i64_be(0)?;
    }

    // max_players
    if version.no_less_than(ProtocolVersion::V1_14)
        && version.less_than(ProtocolVersion::V1_15)
    {
        // 1.14 only: VarInt
        buf.write_var_int(&VarInt(1))?;
    } else {
        // 1.7–1.13 and 1.15: u8
        buf.write_u8(1)?;
    }

    // level_type
    buf.write_string("default")?;

    // view_distance added in 1.14
    if version.no_less_than(ProtocolVersion::V1_14) {
        buf.write_var_int(&VarInt(2))?;
    }

    // reduced_debug_info added in 1.8
    if version.no_less_than(ProtocolVersion::V1_8) {
        buf.write_bool(false)?;
    }

    // enable_respawn_screen added in 1.15
    if version.no_less_than(ProtocolVersion::V1_15) {
        buf.write_bool(false)?;
    }

    Ok(buf)
}

/// Raw CSetContainerContent: window 0, 46 empty slots.
async fn send_clear_inventory(
    client: &mut ClientBridge,
    version: ProtocolVersion,
) -> Result<(), CoreError> {
    let packet_id = container_set_content_packet_id(version);

    let mut buf = Vec::with_capacity(96);
    infrarust_protocol::chunk::write_varint(&mut buf, 0);  // window_id
    infrarust_protocol::chunk::write_varint(&mut buf, 0);  // state_id
    infrarust_protocol::chunk::write_varint(&mut buf, 46); // slot_count
    for _ in 0..46 {
        infrarust_protocol::chunk::write_varint(&mut buf, 0); // empty slot
    }
    infrarust_protocol::chunk::write_varint(&mut buf, 0);  // carried_item

    let frame = PacketFrame {
        id: packet_id,
        payload: Bytes::from(buf),
    };
    client.write_frame(&frame).await?;
    Ok(())
}

fn container_set_content_packet_id(version: ProtocolVersion) -> i32 {
    if version.no_less_than(ProtocolVersion::V1_21_5) {
        0x12
    } else {
        0x13
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_pre_1_16_join_game_v1_7() {
        let result = build_limbo_join_game(ProtocolVersion::V1_7_2);
        assert!(result.is_ok(), "JoinGame 1.7 should build: {:?}", result.err());
        let pkt = result.unwrap();
        assert_eq!(pkt.entity_id, 0);
        let raw = pkt.raw_payload.expect("should have raw_payload");
        assert_eq!(raw.len(), 12);
    }

    #[test]
    fn test_pre_1_16_join_game_v1_8() {
        let result = build_limbo_join_game(ProtocolVersion::V1_8);
        assert!(result.is_ok(), "JoinGame 1.8 should build: {:?}", result.err());
        let pkt = result.unwrap();
        let raw = pkt.raw_payload.expect("should have raw_payload");
        assert_eq!(raw.len(), 13);
    }

    #[test]
    fn test_pre_1_16_join_game_v1_9() {
        let result = build_limbo_join_game(ProtocolVersion::V1_9);
        assert!(result.is_ok(), "JoinGame 1.9 should build: {:?}", result.err());
        let pkt = result.unwrap();
        let raw = pkt.raw_payload.expect("should have raw_payload");
        assert_eq!(raw.len(), 16);
    }

    #[test]
    fn test_pre_1_16_join_game_v1_14() {
        let result = build_limbo_join_game(ProtocolVersion::V1_14);
        assert!(result.is_ok(), "JoinGame 1.14 should build: {:?}", result.err());
        let pkt = result.unwrap();
        let raw = pkt.raw_payload.expect("should have raw_payload");
        assert_eq!(raw.len(), 16);
    }

    #[test]
    fn test_pre_1_16_join_game_v1_15() {
        let result = build_limbo_join_game(ProtocolVersion::V1_15);
        assert!(result.is_ok(), "JoinGame 1.15 should build: {:?}", result.err());
        let pkt = result.unwrap();
        let raw = pkt.raw_payload.expect("should have raw_payload");
        assert_eq!(raw.len(), 25);
    }

    #[test]
    fn test_join_game_1_16_to_1_20_1_still_errors() {
        for version in [ProtocolVersion::V1_16, ProtocolVersion::V1_19, ProtocolVersion::V1_20] {
            let result = build_limbo_join_game(version);
            assert!(result.is_err(), "version {:?} should error", version);
        }
    }

    #[test]
    fn test_join_game_1_20_2_plus_ok() {
        for version in [ProtocolVersion::V1_20_2, ProtocolVersion::V1_21, ProtocolVersion::V1_21_5] {
            let result = build_limbo_join_game(version);
            assert!(result.is_ok(), "version {:?} should succeed: {:?}", version, result.err());
            assert!(result.unwrap().raw_payload.is_none());
        }
    }

    #[test]
    fn test_player_position_y_pre_1_16() {
        let pos = limbo_player_position(ProtocolVersion::V1_8);
        assert!((pos.y - 400.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_player_position_y_post_1_16() {
        let pos = limbo_player_position(ProtocolVersion::V1_21);
        assert!((pos.y - 64.0).abs() < f64::EPSILON);
    }
}
