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
use infrarust_protocol::packets::play::game_event::{CGameEvent, START_WAITING_CHUNKS};
use infrarust_protocol::packets::play::join_game::CJoinGame;
use infrarust_protocol::packets::play::player_position::CSynchronizePlayerPosition;
use infrarust_protocol::packets::play::spawn_position::CSetDefaultSpawnPosition;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use super::chunk::build_chunk_data_frame;
use crate::error::CoreError;
use crate::player::packets::encode_packet;
use crate::session::client_bridge::ClientBridge;

/// The End dimension — used for limbo to avoid same-dimension issues with overworld.
const LIMBO_DIMENSION_NAME: &str = "minecraft:the_end";
const LIMBO_DIMENSION_ID: i32 = 2;
/// The End has 16 sections (height 0-255).
const LIMBO_NUM_SECTIONS: usize = 16;

/// Sends the spawn sequence that makes the client enter the limbo world.
///
/// # Arguments
/// - `client` -- the client bridge to write packets to.
/// - `version` -- the client's protocol version.
/// - `registry` -- the packet registry for encoding.
/// - `needs_join_game` -- `true` for fresh joins (send `JoinGame` first),
///   `false` when switching into limbo from an existing backend (the client
///   is already in Play state).
///
/// # Version branches
///
/// **1.20.3+ (protocol >= 765) with JoinGame**: full sequence including
/// `CJoinGame`, spawn position, player position, center chunk, game event,
/// chunk batch, and chunk data.
///
/// **1.20.3+ without JoinGame**: abbreviated sequence for server-switch.
///
/// **Pre-1.20.3 with JoinGame**: `CJoinGame` + position + chunk.
///
/// **Pre-1.20.3 without JoinGame**: position + chunk only.
///
/// # Errors
/// Returns [`CoreError`] if packet encoding or writing fails, or if
/// pre-1.20.2 `JoinGame` construction is attempted (not yet supported).
pub(crate) async fn send_spawn_sequence(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
    needs_join_game: bool,
) -> Result<(), CoreError> {
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

    // Clear player inventory (all 46 slots empty)
    send_clear_inventory(client, version).await
}

/// 1.20.3+ fresh join -- full spawn sequence with `CJoinGame`.
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

/// 1.20.3+ switch into limbo (no JoinGame -- client already in Play state).
async fn send_modern_switch(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    send_modern_chunk_setup(client, version, registry).await?;
    send_player_position(client, version, registry).await
}

/// Pre-1.20.3 fresh join -- JoinGame + position + chunk.
async fn send_legacy_with_join(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    send_join_game(client, version, registry).await?;
    send_player_position(client, version, registry).await?;
    send_chunk(client, version, registry).await
}

/// Pre-1.20.3 switch into limbo (no JoinGame).
async fn send_legacy_switch(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    send_player_position(client, version, registry).await?;
    send_chunk(client, version, registry).await
}

// -- Packet helpers (each sends a single logical unit) -------------------------

/// Sends JoinGame for the limbo world.
async fn send_join_game(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let join = build_limbo_join_game(version)?;
    let frame = encode_packet(&join, version, registry)?;
    client.write_frame(&frame).await
}

/// Sends the default spawn position for limbo.
async fn send_spawn_position(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let spawn = CSetDefaultSpawnPosition::at_in(LIMBO_DIMENSION_NAME, 0, 64, 0, 0.0);
    let frame = encode_packet(&spawn, version, registry)?;
    client.write_frame(&frame).await
}

/// Sends the player position / teleport packet for limbo spawn.
async fn send_player_position(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let pos = limbo_player_position();
    let frame = encode_packet(&pos, version, registry)?;
    client.write_frame(&frame).await
}

/// Sends a single empty chunk at (0, 0).
async fn send_chunk(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let frame = build_chunk_data_frame(0, 0, LIMBO_NUM_SECTIONS, version, registry)?;
    client.write_frame(&frame).await
}

/// Sends the full modern chunk setup sequence (1.20.3+):
/// center chunk, game event, chunk batch start, chunk data, chunk batch finished.
async fn send_modern_chunk_setup(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    // Center chunk at origin
    let center = CSetCenterChunk { chunk_x: 0, chunk_z: 0 };
    let frame = encode_packet(&center, version, registry)?;
    client.write_frame(&frame).await?;

    // Game event -- start waiting for chunks
    let event = CGameEvent { event: START_WAITING_CHUNKS, value: 0.0 };
    let frame = encode_packet(&event, version, registry)?;
    client.write_frame(&frame).await?;

    // Chunk batch start
    let frame = encode_packet(&CChunkBatchStart, version, registry)?;
    client.write_frame(&frame).await?;

    // Chunk data
    send_chunk(client, version, registry).await?;

    // Chunk batch finished
    let batch_done = CChunkBatchFinished { batch_size: 1 };
    let frame = encode_packet(&batch_done, version, registry)?;
    client.write_frame(&frame).await
}

// -- Helpers ------------------------------------------------------------------

/// Builds the synchronize-player-position packet for limbo spawn (0, 64, 0).
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

/// Builds a minimal `CJoinGame` for the limbo world.
///
/// Only 1.20.2+ is supported (structured fields). For pre-1.20.2, the packet
/// requires `raw_payload` bytes whose format is complex and version-dependent;
/// support will be added later when needed.
fn build_limbo_join_game(version: ProtocolVersion) -> Result<CJoinGame, CoreError> {
    if version.less_than(ProtocolVersion::V1_20_2) {
        return Err(CoreError::Other(
            "limbo JoinGame construction for pre-1.20.2 is not yet implemented".to_string(),
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

/// Sends a `CSetContainerContent` to clear the player's inventory.
///
/// The packet is built as raw bytes since we don't need a full Packet impl.
/// Player inventory = window 0, 46 slots, all empty (VarInt(0) per slot).
async fn send_clear_inventory(
    client: &mut ClientBridge,
    version: ProtocolVersion,
) -> Result<(), CoreError> {
    let packet_id = container_set_content_packet_id(version);

    let mut buf = Vec::with_capacity(96);
    // window_id = 0 (player inventory)
    super::chunk::write_varint(&mut buf, 0);
    // state_id = 0
    super::chunk::write_varint(&mut buf, 0);
    // slot_count = 46
    super::chunk::write_varint(&mut buf, 46);
    // 46 empty slots: each is VarInt(0) = "no item"
    for _ in 0..46 {
        super::chunk::write_varint(&mut buf, 0);
    }
    // carried_item = empty
    super::chunk::write_varint(&mut buf, 0);

    let frame = PacketFrame {
        id: packet_id,
        payload: Bytes::from(buf),
    };
    client.write_frame(&frame).await?;
    Ok(())
}

/// Returns the CSetContainerContent packet ID for a given version.
fn container_set_content_packet_id(version: ProtocolVersion) -> i32 {
    if version.no_less_than(ProtocolVersion::V1_21_5) {
        0x12
    } else {
        0x13
    }
}
