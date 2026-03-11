use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// Respawn packet (Clientbound).
///
/// Sent when a player changes dimension (Overworld -> Nether, server switch).
/// The proxy uses this for server switching: sending a fake Respawn makes
/// the client think it changed dimension.
///
/// Strategy: full parse for >= 1.20.2, opaque for older versions.
#[derive(Debug, Clone)]
pub struct CRespawn {
    /// Dimension ID (VarInt) for 1.20.5+.
    pub dimension: i32,
    pub level_name: String,
    pub hashed_seed: i64,
    pub gamemode: u8,
    pub previous_gamemode: i8,
    pub is_debug: bool,
    pub is_flat: bool,
    /// Bitmask of data to keep across respawn (1.16+).
    pub data_to_keep: u8,
    pub death_dimension: Option<String>,
    pub death_position: Option<i64>,
    pub portal_cooldown: i32,
    pub sea_level: i32,
    /// Opaque payload for pre-1.20.2 versions.
    pub raw_payload: Option<Vec<u8>>,
}

impl Default for CRespawn {
    fn default() -> Self {
        Self {
            dimension: 0,
            level_name: String::new(),
            hashed_seed: 0,
            gamemode: 0,
            previous_gamemode: -1,
            is_debug: false,
            is_flat: false,
            data_to_keep: 0,
            death_dimension: None,
            death_position: None,
            portal_cooldown: 0,
            sea_level: 63,
            raw_payload: None,
        }
    }
}

impl Packet for CRespawn {
    const NAME: &'static str = "CRespawn";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        if version.less_than(ProtocolVersion::V1_20_2) {
            let raw_payload = r.read_remaining()?;
            Ok(Self {
                raw_payload: Some(raw_payload),
                ..Default::default()
            })
        } else {
            decode_1_20_2_up(r, version)
        }
    }

    fn encode(
        &self,
        w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        if let Some(raw) = &self.raw_payload {
            w.write_all(raw)?;
            return Ok(());
        }
        encode_1_20_2_up(self, w, version)
    }
}

/// Decodes Respawn for 1.20.2+ (follows Velocity's RespawnPacket pattern).
fn decode_1_20_2_up(
    r: &mut &[u8],
    version: ProtocolVersion,
) -> ProtocolResult<CRespawn> {
    // Dimension: VarInt for 1.20.5+, String for 1.20.2–1.20.3
    let dimension = if version.no_less_than(ProtocolVersion::V1_20_5) {
        r.read_var_int()?.0
    } else {
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

    // data_to_keep: read at the END for 1.20.2+
    let data_to_keep = r.read_u8()?;

    Ok(CRespawn {
        dimension,
        level_name,
        hashed_seed,
        gamemode,
        previous_gamemode,
        is_debug,
        is_flat,
        data_to_keep,
        death_dimension,
        death_position,
        portal_cooldown,
        sea_level,
        raw_payload: None,
    })
}

/// Encodes Respawn for 1.20.2+.
fn encode_1_20_2_up(
    pkt: &CRespawn,
    mut w: &mut (impl std::io::Write + ?Sized),
    version: ProtocolVersion,
) -> ProtocolResult<()> {
    if version.no_less_than(ProtocolVersion::V1_20_5) {
        w.write_var_int(&VarInt(pkt.dimension))?;
    } else {
        w.write_string("minecraft:overworld")?;
    }

    w.write_string(&pkt.level_name)?;
    w.write_i64_be(pkt.hashed_seed)?;
    w.write_u8(pkt.gamemode)?;
    w.write_i8(pkt.previous_gamemode)?;
    w.write_bool(pkt.is_debug)?;
    w.write_bool(pkt.is_flat)?;

    super::common::encode_death_location(w, &pkt.death_dimension, pkt.death_position)?;
    super::common::encode_world_info(w, pkt.portal_cooldown, pkt.sea_level, version)?;

    // data_to_keep at END for 1.20.2+
    w.write_u8(pkt.data_to_keep)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_version(packet: &CRespawn, version: ProtocolVersion) -> CRespawn {
        let mut buf = Vec::new();
        packet.encode(&mut buf, version).unwrap();
        CRespawn::decode(&mut buf.as_slice(), version).unwrap()
    }

    #[test]
    fn test_respawn_round_trip() {
        let pkt = CRespawn {
            dimension: 1,
            level_name: "minecraft:the_nether".to_string(),
            hashed_seed: 987654321,
            gamemode: 0,
            previous_gamemode: 1,
            is_debug: false,
            is_flat: false,
            data_to_keep: 0x03,
            death_dimension: None,
            death_position: None,
            portal_cooldown: 0,
            sea_level: 63,
            raw_payload: None,
        };
        let decoded = round_trip_version(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.dimension, 1);
        assert_eq!(decoded.level_name, "minecraft:the_nether");
        assert_eq!(decoded.hashed_seed, 987654321);
        assert_eq!(decoded.gamemode, 0);
        assert_eq!(decoded.previous_gamemode, 1);
        assert_eq!(decoded.data_to_keep, 0x03);
    }

    #[test]
    fn test_respawn_with_death_location() {
        let pkt = CRespawn {
            dimension: 0,
            level_name: "minecraft:overworld".to_string(),
            hashed_seed: 0,
            gamemode: 0,
            previous_gamemode: -1,
            is_debug: false,
            is_flat: false,
            data_to_keep: 0,
            death_dimension: Some("minecraft:overworld".to_string()),
            death_position: Some(12345678),
            portal_cooldown: 10,
            sea_level: 63,
            raw_payload: None,
        };
        let decoded = round_trip_version(&pkt, ProtocolVersion::V1_20_5);
        assert_eq!(
            decoded.death_dimension.as_deref(),
            Some("minecraft:overworld")
        );
        assert_eq!(decoded.death_position, Some(12345678));
        assert_eq!(decoded.portal_cooldown, 10);
    }

    #[test]
    fn test_respawn_opaque_pre_1_20_2() {
        let raw = vec![0xAA, 0xBB, 0xCC, 0xDD];
        let pkt = CRespawn {
            raw_payload: Some(raw.clone()),
            ..Default::default()
        };

        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_19).unwrap();
        let decoded = CRespawn::decode(&mut buf.as_slice(), ProtocolVersion::V1_19).unwrap();
        assert_eq!(decoded.raw_payload, Some(raw));
    }
}
