//! Spawn sequence -- version-branched packet sequence for entering limbo.
//!
//! Sends the minimal set of packets needed for the client to enter the
//! limbo world (an empty flat void). The exact sequence depends on the
//! protocol version and whether this is a fresh join or a switch into limbo
//! from an existing backend connection.

use bytes::Bytes;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::play::center_chunk::CSetCenterChunk;
use infrarust_protocol::packets::play::chunk_batch::{CChunkBatchFinished, CChunkBatchStart};
use infrarust_protocol::packets::play::dimension::DimensionInfo;
use infrarust_protocol::packets::play::game_event::{CGameEvent, START_WAITING_CHUNKS};
use infrarust_protocol::packets::play::join_game::CJoinGame;
use infrarust_protocol::packets::play::player_position::CSynchronizePlayerPosition;
use infrarust_protocol::packets::play::respawn_switch;
use infrarust_protocol::packets::play::spawn_position::CSetDefaultSpawnPosition;
use infrarust_protocol::codec::{McBufWriteExt, VarInt};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use infrarust_protocol::chunk::build_chunk_data_frame;
use crate::error::CoreError;
use crate::player::packets::encode_packet;
use crate::session::client_bridge::ClientBridge;

const LIMBO_DIMENSION_NAME: &str = "minecraft:the_end";
const LIMBO_DIMENSION_ID: i32 = 0;
/// Legacy dimension ID for The End (pre-1.16 uses integer IDs: -1=Nether, 0=Overworld, 1=End).
const LIMBO_DIMENSION_END: i32 = 1;
const LIMBO_NUM_SECTIONS: usize = 16;

pub(crate) async fn send_spawn_sequence(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
    needs_join_game: bool,
) -> Result<(), CoreError> {
    if version.less_than(ProtocolVersion::V1_16) {
        if needs_join_game {
            send_join_game(client, version, registry).await?;
            send_limbo_respawn_trick(client, version, registry).await?;
        }
        return send_player_position_at(client, version, registry, 400.0).await;
    }

    let is_modern = version.no_less_than(ProtocolVersion::V1_20_3);

    if is_modern && needs_join_game {
        send_modern_with_join(client, version, registry).await?;
    } else if is_modern {
        send_modern_switch(client, version, registry).await?;
    } else if needs_join_game {
        send_legacy_with_join(client, version, registry).await?;
    } else {
        send_legacy_switch(client, version, registry).await?;
    }

    send_clear_inventory(client, version).await
}

async fn send_modern_with_join(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    send_join_game(client, version, registry).await?;
    send_spawn_position(client, version, registry).await?;
    send_player_position_at(client, version, registry, 64.0).await?;
    send_modern_chunk_setup(client, version, registry).await
}

async fn send_modern_switch(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    send_modern_chunk_setup(client, version, registry).await?;
    send_player_position_at(client, version, registry, 64.0).await
}

async fn send_legacy_with_join(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    send_join_game(client, version, registry).await?;
    send_player_position_at(client, version, registry, 64.0).await?;
    send_chunk(client, version, registry).await
}

async fn send_legacy_switch(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    send_player_position_at(client, version, registry, 64.0).await?;
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

async fn send_player_position_at(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
    y: f64,
) -> Result<(), CoreError> {
    let pos = CSynchronizePlayerPosition { y, ..limbo_player_position() };
    let frame = encode_packet(&pos, version, registry)?;
    client.write_frame(&frame).await
}

async fn send_limbo_respawn_trick(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let temp_dim = DimensionInfo::Legacy(0);
    let limbo_dim = DimensionInfo::Legacy(LIMBO_DIMENSION_END);

    for dim in [&temp_dim, &limbo_dim] {
        let respawn = respawn_switch::for_switch(dim, version);
        let frame = encode_packet(&respawn, version, registry)?;
        client.write_frame(&frame).await?;
    }
    Ok(())
}

async fn send_chunk(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    _registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let frame = build_chunk_data_frame(0, 0, LIMBO_NUM_SECTIONS, version)?;
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

fn limbo_player_position() -> CSynchronizePlayerPosition {
    CSynchronizePlayerPosition {
        x: 0.0,
        y: 64.0,
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
        return build_limbo_join_game_pre16(version);
    }

    if version.less_than(ProtocolVersion::V1_20_2) {
        return Err(CoreError::Other(
            "limbo JoinGame construction for 1.16-1.20.1 is not yet implemented".to_string(),
        ));
    }

    Ok(CJoinGame {
        entity_id: 0,
        is_hardcore: false,
        gamemode: 2,
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
        sea_level: 0,
        enforces_secure_chat: false,
        death_dimension: None,
        death_position: None,
        raw_payload: None,
    })
}

fn build_limbo_join_game_pre16(version: ProtocolVersion) -> Result<CJoinGame, CoreError> {
    let mut buf: Vec<u8> = Vec::with_capacity(32);

    if version.less_than(ProtocolVersion::V1_9) {
        // 1.7 / 1.8
        buf.write_u8(2)?;                           // gamemode
        buf.write_i8(LIMBO_DIMENSION_END as i8)?;   // dimension (i8)
        buf.write_u8(0)?;                           // difficulty
        buf.write_u8(1)?;                           // max_players
        buf.write_string("default")?;               // level_type
        if version.no_less_than(ProtocolVersion::V1_8) {
            buf.write_bool(false)?;                 // reduced_debug_info
        }
    } else if version.less_than(ProtocolVersion::V1_14) {
        // 1.9–1.13
        buf.write_u8(2)?;                           // gamemode
        buf.write_i32_be(LIMBO_DIMENSION_END)?;     // dimension (i32)
        buf.write_u8(0)?;                           // difficulty
        buf.write_u8(1)?;                           // max_players
        buf.write_string("default")?;               // level_type
        buf.write_bool(false)?;                     // reduced_debug_info
    } else if version.less_than(ProtocolVersion::V1_15) {
        // 1.14 — no difficulty, max_players is VarInt, view_distance added
        buf.write_u8(2)?;                           // gamemode
        buf.write_i32_be(LIMBO_DIMENSION_END)?;     // dimension
        buf.write_var_int(&VarInt(1))?;             // max_players
        buf.write_string("default")?;               // level_type
        buf.write_var_int(&VarInt(2))?;             // view_distance
        buf.write_bool(false)?;                     // reduced_debug_info
    } else {
        // 1.15 — hashed_seed and enable_respawn_screen added
        buf.write_u8(2)?;                           // gamemode
        buf.write_i32_be(LIMBO_DIMENSION_END)?;     // dimension
        buf.write_i64_be(0)?;                       // hashed_seed
        buf.write_u8(1)?;                           // max_players
        buf.write_string("default")?;               // level_type
        buf.write_var_int(&VarInt(2))?;             // view_distance
        buf.write_bool(false)?;                     // reduced_debug_info
        buf.write_bool(true)?;                      // enable_respawn_screen
    }

    Ok(CJoinGame {
        entity_id: 0,
        raw_payload: Some(buf),
        ..Default::default()
    })
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
    use infrarust_protocol::packets::Packet;

    #[test]
    fn test_build_join_game_1_7_2() {
        let join = build_limbo_join_game(ProtocolVersion::V1_7_2).unwrap();
        assert_eq!(join.entity_id, 0);
        assert!(join.raw_payload.is_some());
        let mut buf = Vec::new();
        join.encode(&mut buf, ProtocolVersion::V1_7_2).unwrap();
        let decoded = CJoinGame::decode(&mut buf.as_slice(), ProtocolVersion::V1_7_2).unwrap();
        assert_eq!(decoded.entity_id, 0);
        assert_eq!(decoded.raw_payload, join.raw_payload);
    }

    #[test]
    fn test_build_join_game_1_8() {
        let join = build_limbo_join_game(ProtocolVersion::V1_8).unwrap();
        let join_17 = build_limbo_join_game(ProtocolVersion::V1_7_2).unwrap();
        assert_eq!(
            join.raw_payload.as_ref().unwrap().len(),
            join_17.raw_payload.as_ref().unwrap().len() + 1
        );
    }

    #[test]
    fn test_build_join_game_1_9() {
        let join = build_limbo_join_game(ProtocolVersion::V1_9).unwrap();
        let payload = join.raw_payload.as_ref().unwrap();
        assert!(payload.len() > 5);
    }

    #[test]
    fn test_build_join_game_1_14() {
        let join = build_limbo_join_game(ProtocolVersion::V1_14).unwrap();
        assert!(join.raw_payload.is_some());
    }

    #[test]
    fn test_build_join_game_1_15() {
        let join = build_limbo_join_game(ProtocolVersion::V1_15).unwrap();
        let payload = join.raw_payload.as_ref().unwrap();
        assert!(payload.len() > 15);
    }

    #[test]
    fn test_build_join_game_1_16_gap_errors() {
        let result = build_limbo_join_game(ProtocolVersion::V1_16);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_join_game_modern() {
        let join = build_limbo_join_game(ProtocolVersion::V1_21).unwrap();
        assert!(join.raw_payload.is_none());
        assert_eq!(join.gamemode, 2);
    }
}
