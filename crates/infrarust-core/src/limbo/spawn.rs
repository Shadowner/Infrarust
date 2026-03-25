//! Spawn sequence -- version-branched packet sequence for entering limbo.
//!
//! Sends the minimal set of packets needed for the client to enter the
//! limbo world (an empty flat void). The exact sequence depends on the
//! protocol version and whether this is a fresh join or a switch into limbo
//! from an existing backend connection.

use bytes::Bytes;
use infrarust_protocol::codec::{McBufWriteExt, VarInt};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::play::center_chunk::CSetCenterChunk;
use infrarust_protocol::packets::play::chunk_batch::{CChunkBatchFinished, CChunkBatchStart};
use infrarust_protocol::packets::play::dimension::DimensionInfo;
use infrarust_protocol::packets::play::game_event::{CGameEvent, START_WAITING_CHUNKS};
use infrarust_protocol::packets::play::join_game::CJoinGame;
use infrarust_protocol::packets::play::player_position::CSynchronizePlayerPosition;
use infrarust_protocol::packets::play::respawn::CRespawn;
use infrarust_protocol::packets::play::respawn_switch;
use infrarust_protocol::packets::play::spawn_position::CSetDefaultSpawnPosition;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::error::CoreError;
use crate::player::packets::encode_packet;
use crate::session::client_bridge::ClientBridge;
use infrarust_protocol::chunk::build_chunk_data_frame;

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
    if version.no_less_than(ProtocolVersion::V1_16) {
        send_limbo_respawn(
            client,
            &DimensionInfo::Named("minecraft:overworld".to_string()),
            version,
            registry,
        )
        .await?;
        send_limbo_respawn(
            client,
            &DimensionInfo::Named(LIMBO_DIMENSION_NAME.to_string()),
            version,
            registry,
        )
        .await?;
    }
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
    let center = CSetCenterChunk {
        chunk_x: 0,
        chunk_z: 0,
    };
    let frame = encode_packet(&center, version, registry)?;
    client.write_frame(&frame).await?;

    let event = CGameEvent {
        event: START_WAITING_CHUNKS,
        value: 0.0,
    };
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
        let raw_payload = build_1_16_to_1_20_1_join_game_payload(version)?;
        return Ok(CJoinGame {
            entity_id: 0,
            raw_payload: Some(raw_payload),
            ..Default::default()
        });
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
fn build_pre_1_16_join_game_payload(version: ProtocolVersion) -> Result<Vec<u8>, CoreError> {
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
    if version.no_less_than(ProtocolVersion::V1_14) && version.less_than(ProtocolVersion::V1_15) {
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

fn build_1_16_to_1_20_1_join_game_payload(version: ProtocolVersion) -> Result<Vec<u8>, CoreError> {
    use super::registry_nbt;

    let pvn = version.0;
    let mut buf = Vec::with_capacity(512);

    // 1.16.2+: is_hardcore as separate bool (1.16.0–1.16.1 encodes it in gamemode byte)
    if pvn >= 751 {
        buf.write_bool(false)?; // is_hardcore
    }

    // gamemode (adventure=2), previous_gamemode (-1)
    buf.write_u8(2)?;
    buf.write_i8(-1)?;

    // dimension_names list (VarInt count + Identifier strings)
    buf.write_var_int(&VarInt(1))?;
    buf.write_string(LIMBO_DIMENSION_NAME)?;

    // registry_codec (NBT compound, named root)
    let registry_codec = registry_nbt::build_registry_codec(pvn);
    buf.extend_from_slice(&registry_codec);

    // 1.16.2–1.18.2: dimension_codec (separate NBT compound for the dimension type)
    if (751..=758).contains(&pvn) {
        let dimension_codec = registry_nbt::build_dimension_codec(pvn);
        buf.extend_from_slice(&dimension_codec);
    }

    // dimension_name / dimension_type identifier
    if pvn >= 759 {
        // 1.19+: "dimension_type" identifier (references registry entry)
        buf.write_string(LIMBO_DIMENSION_NAME)?;
    } else {
        // 1.16–1.18.2: "dimension_name" identifier
        buf.write_string(LIMBO_DIMENSION_NAME)?;
    }

    // world_name
    buf.write_string(LIMBO_DIMENSION_NAME)?;

    // hashed_seed
    buf.write_i64_be(0)?;

    // max_players (VarInt)
    buf.write_var_int(&VarInt(1))?;

    // view_distance (VarInt)
    buf.write_var_int(&VarInt(2))?;

    // 1.18+ (pvn >= 757): simulation_distance
    if pvn >= 757 {
        buf.write_var_int(&VarInt(2))?;
    }

    // reduced_debug_info, enable_respawn_screen, is_debug, is_flat
    buf.write_bool(false)?; // reduced_debug_info
    buf.write_bool(true)?; // enable_respawn_screen
    buf.write_bool(false)?; // is_debug
    buf.write_bool(false)?; // is_flat

    // 1.19+ (pvn >= 759): death location (optional)
    if pvn >= 759 {
        buf.write_bool(false)?; // has_death_location = false
    }

    // 1.19.4+ (pvn >= 762): portal_cooldown
    if pvn >= 762 {
        buf.write_var_int(&VarInt(0))?;
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

    if version.no_less_than(ProtocolVersion::V1_17) {
        // 1.17.1+ format: window_id(u8) + state_id(VarInt) + count(VarInt) + slots + carried(Slot)
        buf.push(0); // window_id (u8)
        infrarust_protocol::chunk::write_varint(&mut buf, 0); // state_id
        infrarust_protocol::chunk::write_varint(&mut buf, 46); // slot_count
        for _ in 0..46 {
            // Empty slot: present=false (1.13+) or count=0 (1.20.5+) — both encode as 0x00
            buf.push(0);
        }
        buf.push(0); // carried_item: empty slot
    } else {
        // Pre-1.17.1 format: window_id(u8) + count(i16 BE) + slots
        buf.push(0); // window_id (u8)
        buf.extend_from_slice(&46_i16.to_be_bytes()); // count (i16)
        for _ in 0..46 {
            buf.push(0); // empty slot: present=false
        }
    }

    let frame = PacketFrame {
        id: packet_id,
        payload: Bytes::from(buf),
    };
    client.write_frame(&frame).await?;
    Ok(())
}

fn container_set_content_packet_id(version: ProtocolVersion) -> i32 {
    let pvn = version.0;
    match pvn {
        // 1.21.5 (770)+
        770.. => 0x12,
        // 1.20.2 (764) .. 1.21.4 (769)
        764..=769 => 0x13,
        // 1.19.4 (762) .. 1.20.1 (763)
        762..=763 => 0x12,
        // 1.19.3 (761)
        761 => 0x11,
        // 1.19.1 (760)
        760 => 0x12,
        // 1.19 (759)
        759 => 0x13,
        // 1.17 (755) .. 1.18.2 (758)
        755..=758 => 0x14,
        // 1.16.2 (751) .. 1.16.4 (754)
        751..=754 => 0x13,
        // 1.16 (735) .. 1.16.1 (736)
        735..=750 => 0x14,
        // Pre-1.16 is skipped by caller; fallback for safety
        _ => 0x13,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_pre_1_16_join_game_v1_7() {
        let result = build_limbo_join_game(ProtocolVersion::V1_7_2);
        assert!(
            result.is_ok(),
            "JoinGame 1.7 should build: {:?}",
            result.err()
        );
        let pkt = result.unwrap();
        assert_eq!(pkt.entity_id, 0);
        let raw = pkt.raw_payload.expect("should have raw_payload");
        assert_eq!(raw.len(), 12);
    }

    #[test]
    fn test_pre_1_16_join_game_v1_8() {
        let result = build_limbo_join_game(ProtocolVersion::V1_8);
        assert!(
            result.is_ok(),
            "JoinGame 1.8 should build: {:?}",
            result.err()
        );
        let pkt = result.unwrap();
        let raw = pkt.raw_payload.expect("should have raw_payload");
        assert_eq!(raw.len(), 13);
    }

    #[test]
    fn test_pre_1_16_join_game_v1_9() {
        let result = build_limbo_join_game(ProtocolVersion::V1_9);
        assert!(
            result.is_ok(),
            "JoinGame 1.9 should build: {:?}",
            result.err()
        );
        let pkt = result.unwrap();
        let raw = pkt.raw_payload.expect("should have raw_payload");
        assert_eq!(raw.len(), 16);
    }

    #[test]
    fn test_pre_1_16_join_game_v1_14() {
        let result = build_limbo_join_game(ProtocolVersion::V1_14);
        assert!(
            result.is_ok(),
            "JoinGame 1.14 should build: {:?}",
            result.err()
        );
        let pkt = result.unwrap();
        let raw = pkt.raw_payload.expect("should have raw_payload");
        assert_eq!(raw.len(), 16);
    }

    #[test]
    fn test_pre_1_16_join_game_v1_15() {
        let result = build_limbo_join_game(ProtocolVersion::V1_15);
        assert!(
            result.is_ok(),
            "JoinGame 1.15 should build: {:?}",
            result.err()
        );
        let pkt = result.unwrap();
        let raw = pkt.raw_payload.expect("should have raw_payload");
        assert_eq!(raw.len(), 25);
    }

    #[test]
    fn test_join_game_1_16_to_1_20_1_ok() {
        for version in [
            ProtocolVersion::V1_16,
            ProtocolVersion::V1_16_2,
            ProtocolVersion::V1_17,
            ProtocolVersion::V1_18,
            ProtocolVersion::V1_19,
            ProtocolVersion::V1_19_3,
            ProtocolVersion::V1_19_4,
            ProtocolVersion::V1_20,
        ] {
            let result = build_limbo_join_game(version);
            assert!(
                result.is_ok(),
                "version {:?} should succeed: {:?}",
                version,
                result.err()
            );
            let pkt = result.unwrap();
            assert_eq!(pkt.entity_id, 0);
            assert!(
                pkt.raw_payload.is_some(),
                "version {:?} should use raw_payload",
                version
            );
        }
    }

    #[test]
    fn test_join_game_1_20_2_plus_ok() {
        for version in [
            ProtocolVersion::V1_20_2,
            ProtocolVersion::V1_21,
            ProtocolVersion::V1_21_5,
        ] {
            let result = build_limbo_join_game(version);
            assert!(
                result.is_ok(),
                "version {:?} should succeed: {:?}",
                version,
                result.err()
            );
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
