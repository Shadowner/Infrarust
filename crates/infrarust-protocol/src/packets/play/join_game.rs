use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// Join game packet (Clientbound).
///
/// Sent by the server when the player joins or is transferred. The proxy
/// intercepts this to learn the entity ID, gamemode, and dimension info
/// needed for server switching and Virtual Backend.
///
/// This is the most complex packet in the protocol with 3 format eras.
/// Strategy:
/// - **1.20.2+**: All fields are fully parsed.
/// - **Pre-1.20.2**: Only `entity_id` is parsed; everything else is stored
///   in `raw_payload` for opaque forwarding.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // Protocol-defined fields, cannot refactor
pub struct CJoinGame {
    pub entity_id: i32,
    pub is_hardcore: bool,
    pub gamemode: u8,
    pub previous_gamemode: i8,
    pub max_players: i32,
    pub view_distance: i32,
    pub simulation_distance: i32,
    pub reduced_debug_info: bool,
    pub enable_respawn_screen: bool,
    pub do_limited_crafting: bool,
    pub level_names: Vec<String>,
    pub level_name: String,
    pub hashed_seed: i64,
    pub is_debug: bool,
    pub is_flat: bool,
    /// Dimension ID (as `VarInt`) for 1.20.5+, or dimension key index for 1.20.2-1.20.3.
    pub dimension: i32,
    pub portal_cooldown: i32,
    pub sea_level: i32,
    pub enforces_secure_chat: bool,
    /// Death location (1.19+): dimension identifier.
    pub death_dimension: Option<String>,
    /// Death location (1.19+): packed position.
    pub death_position: Option<i64>,
    /// Opaque payload for pre-1.20.2 versions (everything after `entity_id`).
    /// When set, encode writes entity ID + raw payload verbatim.
    pub raw_payload: Option<Vec<u8>>,
}

impl Default for CJoinGame {
    fn default() -> Self {
        Self {
            entity_id: 0,
            is_hardcore: false,
            gamemode: 0,
            previous_gamemode: -1,
            max_players: 20,
            view_distance: 10,
            simulation_distance: 10,
            reduced_debug_info: false,
            enable_respawn_screen: true,
            do_limited_crafting: false,
            level_names: Vec::new(),
            level_name: String::new(),
            hashed_seed: 0,

            is_debug: false,
            is_flat: false,
            dimension: 0,
            portal_cooldown: 0,
            sea_level: 63,
            enforces_secure_chat: false,
            death_dimension: None,
            death_position: None,
            raw_payload: None,
        }
    }
}

impl Packet for CJoinGame {
    const NAME: &'static str = "CJoinGame";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let entity_id = r.read_i32_be()?;

        if version.less_than(ProtocolVersion::V1_20_2) {
            // Store everything else as opaque
            let raw_payload = r.read_remaining()?;
            Ok(Self {
                entity_id,
                raw_payload: Some(raw_payload),
                ..Default::default()
            })
        } else {
            decode_1_20_2_up(r, entity_id, version)
        }
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_i32_be(self.entity_id)?;

        if let Some(ref raw) = self.raw_payload {
            w.write_all(raw)?;
            return Ok(());
        }

        encode_1_20_2_up(self, w, version)
    }
}

/// Decodes `JoinGame` for 1.20.2+ (Velocity's decode1202Up pattern).
fn decode_1_20_2_up(
    r: &mut &[u8],
    entity_id: i32,
    version: ProtocolVersion,
) -> ProtocolResult<CJoinGame> {
    let is_hardcore = r.read_bool()?;

    // Level names
    let level_count = r.read_var_int()?.0 as usize;
    let mut level_names = Vec::with_capacity(level_count.min(64));
    for _ in 0..level_count {
        level_names.push(r.read_string()?);
    }

    let max_players = r.read_var_int()?.0;
    let view_distance = r.read_var_int()?.0;
    let simulation_distance = r.read_var_int()?.0;
    let reduced_debug_info = r.read_bool()?;
    let enable_respawn_screen = r.read_bool()?;
    let do_limited_crafting = r.read_bool()?;

    // Dimension: VarInt for 1.21.2+, String (identifier) for 1.20.2–1.21.1
    let dimension = if version.no_less_than(ProtocolVersion::V1_21_2) {
        r.read_var_int()?.0
    } else {
        // For 1.20.2–1.21.1, dimension is a String identifier.
        // We store 0 and the string is lost (proxy primarily targets 1.21.2+).
        let _dim_key = r.read_string()?;
        0
    };

    let level_name = r.read_string()?;
    let hashed_seed = r.read_i64_be()?;
    let gamemode = r.read_u8()?;
    let previous_gamemode = r.read_i8()?;
    let is_debug = r.read_bool()?;
    let is_flat = r.read_bool()?;

    let (death_dimension, death_position) = super::common::decode_death_location(r)?;
    let (portal_cooldown, sea_level) = super::common::decode_world_info(r, version)?;

    let enforces_secure_chat = if version.no_less_than(ProtocolVersion::V1_20_5) {
        r.read_bool()?
    } else {
        false
    };

    Ok(CJoinGame {
        entity_id,
        is_hardcore,
        gamemode,
        previous_gamemode,
        max_players,
        view_distance,
        simulation_distance,
        reduced_debug_info,
        enable_respawn_screen,
        do_limited_crafting,
        level_names,
        level_name,
        hashed_seed,
        is_debug,
        is_flat,
        dimension,
        portal_cooldown,
        sea_level,
        enforces_secure_chat,
        death_dimension,
        death_position,
        raw_payload: None,
    })
}

/// Encodes `JoinGame` for 1.20.2+.
fn encode_1_20_2_up(
    pkt: &CJoinGame,
    mut w: &mut (impl std::io::Write + ?Sized),
    version: ProtocolVersion,
) -> ProtocolResult<()> {
    w.write_bool(pkt.is_hardcore)?;

    // Level names
    // Level name count bounded by protocol
    w.write_var_int(&VarInt(pkt.level_names.len() as i32))?;
    for name in &pkt.level_names {
        w.write_string(name)?;
    }

    w.write_var_int(&VarInt(pkt.max_players))?;
    w.write_var_int(&VarInt(pkt.view_distance))?;
    w.write_var_int(&VarInt(pkt.simulation_distance))?;
    w.write_bool(pkt.reduced_debug_info)?;
    w.write_bool(pkt.enable_respawn_screen)?;
    w.write_bool(pkt.do_limited_crafting)?;

    // Dimension: VarInt for 1.21.2+, String identifier for 1.20.2–1.21.1
    if version.no_less_than(ProtocolVersion::V1_21_2) {
        w.write_var_int(&VarInt(pkt.dimension))?;
    } else {
        // 1.20.2–1.21.1: dimension as String identifier
        w.write_string("minecraft:overworld")?;
    }

    w.write_string(&pkt.level_name)?;
    w.write_i64_be(pkt.hashed_seed)?;
    w.write_u8(pkt.gamemode)?;
    w.write_i8(pkt.previous_gamemode)?;
    w.write_bool(pkt.is_debug)?;
    w.write_bool(pkt.is_flat)?;

    super::common::encode_death_location(w, pkt.death_dimension.as_deref(), pkt.death_position)?;
    super::common::encode_world_info(w, pkt.portal_cooldown, pkt.sea_level, version)?;

    if version.no_less_than(ProtocolVersion::V1_20_5) {
        w.write_bool(pkt.enforces_secure_chat)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn round_trip_version(packet: &CJoinGame, version: ProtocolVersion) -> CJoinGame {
        let mut buf = Vec::new();
        packet.encode(&mut buf, version).unwrap();
        CJoinGame::decode(&mut buf.as_slice(), version).unwrap()
    }

    #[test]
    fn test_join_game_round_trip_modern() {
        let pkt = CJoinGame {
            entity_id: 42,
            is_hardcore: true,
            gamemode: 1,
            previous_gamemode: 0,
            max_players: 100,
            view_distance: 16,
            simulation_distance: 12,
            reduced_debug_info: false,
            enable_respawn_screen: true,
            do_limited_crafting: false,
            level_names: vec![
                "minecraft:overworld".to_string(),
                "minecraft:the_nether".to_string(),
            ],
            level_name: "minecraft:overworld".to_string(),
            hashed_seed: 123_456_789,
            is_debug: false,
            is_flat: false,
            dimension: 0,
            portal_cooldown: 20,
            sea_level: 63,
            enforces_secure_chat: true,
            death_dimension: None,
            death_position: None,
            raw_payload: None,
        };
        let decoded = round_trip_version(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.entity_id, 42);
        assert!(decoded.is_hardcore);
        assert_eq!(decoded.gamemode, 1);
        assert_eq!(decoded.previous_gamemode, 0);
        assert_eq!(decoded.max_players, 100);
        assert_eq!(decoded.view_distance, 16);
        assert_eq!(decoded.simulation_distance, 12);
        assert!(decoded.enable_respawn_screen);
        assert_eq!(decoded.level_names.len(), 2);
        assert_eq!(decoded.level_name, "minecraft:overworld");
        assert_eq!(decoded.hashed_seed, 123_456_789);
        assert_eq!(decoded.portal_cooldown, 20);
        assert!(decoded.enforces_secure_chat);
        assert!(decoded.raw_payload.is_none());
    }

    #[test]
    fn test_join_game_round_trip_1_20_2() {
        let pkt = CJoinGame {
            entity_id: 42,
            is_hardcore: true,
            gamemode: 1,
            previous_gamemode: 0,
            max_players: 100,
            view_distance: 16,
            simulation_distance: 12,
            reduced_debug_info: false,
            enable_respawn_screen: true,
            do_limited_crafting: false,
            level_names: vec![
                "minecraft:overworld".to_string(),
                "minecraft:the_nether".to_string(),
            ],
            level_name: "minecraft:overworld".to_string(),
            hashed_seed: 123_456_789,
            is_debug: false,
            is_flat: false,
            dimension: 0,
            portal_cooldown: 20,
            sea_level: 63,
            enforces_secure_chat: true, // intentionally true to verify it is NOT encoded
            death_dimension: None,
            death_position: None,
            raw_payload: None,
        };
        let decoded = round_trip_version(&pkt, ProtocolVersion::V1_20_2);
        assert_eq!(decoded.entity_id, 42);
        assert!(decoded.is_hardcore);
        assert_eq!(decoded.gamemode, 1);
        assert_eq!(decoded.previous_gamemode, 0);
        assert_eq!(decoded.max_players, 100);
        assert_eq!(decoded.view_distance, 16);
        assert_eq!(decoded.simulation_distance, 12);
        assert!(decoded.enable_respawn_screen);
        assert_eq!(decoded.level_names.len(), 2);
        assert_eq!(decoded.level_name, "minecraft:overworld");
        assert_eq!(decoded.hashed_seed, 123_456_789);
        assert_eq!(decoded.portal_cooldown, 20);
        assert!(!decoded.enforces_secure_chat);
        assert!(decoded.raw_payload.is_none());
    }

    #[test]
    fn test_join_game_entity_id_preserved() {
        let pkt = CJoinGame {
            entity_id: -12345,
            ..Default::default()
        };
        // Test with modern version
        let decoded = round_trip_version(&pkt, ProtocolVersion::V1_20_5);
        assert_eq!(decoded.entity_id, -12345);
    }

    #[test]
    fn test_join_game_opaque_data_preserved() {
        // For pre-1.20.2, the packet stores raw_payload
        let raw = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let pkt = CJoinGame {
            entity_id: 99,
            raw_payload: Some(raw.clone()),
            ..Default::default()
        };

        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_19).unwrap();
        let decoded = CJoinGame::decode(&mut buf.as_slice(), ProtocolVersion::V1_19).unwrap();
        assert_eq!(decoded.entity_id, 99);
        assert_eq!(decoded.raw_payload, Some(raw));
    }

    #[test]
    fn test_join_game_death_location() {
        let pkt = CJoinGame {
            entity_id: 1,
            death_dimension: Some("minecraft:the_nether".to_string()),
            death_position: Some(0x0000_0001_0000_0002),
            level_names: vec!["minecraft:overworld".to_string()],
            level_name: "minecraft:overworld".to_string(),
            ..Default::default()
        };
        let decoded = round_trip_version(&pkt, ProtocolVersion::V1_20_5);
        assert_eq!(
            decoded.death_dimension.as_deref(),
            Some("minecraft:the_nether")
        );
        assert_eq!(decoded.death_position, Some(0x0000_0001_0000_0002));
    }

    #[test]
    fn test_join_game_sea_level_v1_21_2() {
        let pkt = CJoinGame {
            entity_id: 1,
            sea_level: 128,
            level_names: vec!["minecraft:overworld".to_string()],
            level_name: "minecraft:overworld".to_string(),
            ..Default::default()
        };
        let decoded = round_trip_version(&pkt, ProtocolVersion::V1_21_2);
        assert_eq!(decoded.sea_level, 128);
    }
}
