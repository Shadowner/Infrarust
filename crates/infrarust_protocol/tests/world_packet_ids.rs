#[allow(dead_code)]
mod common;
use common::assert_ids;
use infrarust_protocol::{
    CChunkData, CGameEvent, CSetCenterChunk, CSetDefaultSpawnPosition, CSynchronizePlayerPosition,
};

const C_GAME_EVENT: &[(i32, Option<i32>)] = &[
    (4, None),
    (5, None),
    (47, None),
    (107, Some(0x1E)),
    (109, Some(0x1E)),
    (110, Some(0x1E)),
    (335, Some(0x1E)),
    (338, Some(0x1E)),
    (340, Some(0x1E)),
    (393, Some(0x20)),
    (477, Some(0x1E)),
    (573, Some(0x1F)),
    (735, Some(0x1E)),
    (751, Some(0x1D)),
    (754, Some(0x1D)),
    (755, Some(0x1E)),
    (757, Some(0x1E)),
    (758, Some(0x1E)),
    (759, Some(0x1B)),
    (760, Some(0x1D)),
    (761, Some(0x1C)),
    (762, Some(0x1F)),
    (763, Some(0x1F)),
    (764, Some(0x20)),
    (765, Some(0x20)),
    (766, Some(0x22)),
    (767, Some(0x22)),
    (768, Some(0x23)),
    (769, Some(0x23)),
    (770, Some(0x22)),
    (771, Some(0x22)),
    (772, Some(0x22)),
    (773, Some(0x26)),
    (774, Some(0x26)),
    (775, Some(0x26)),
    (776, Some(0x26)),
];

const C_SET_CENTER_CHUNK: &[(i32, Option<i32>)] = &[
    (4, None),
    (5, None),
    (47, None),
    (107, None),
    (109, None),
    (110, None),
    (335, None),
    (338, None),
    (340, None),
    (393, None),
    (477, Some(0x40)),
    (573, Some(0x41)),
    (735, Some(0x40)),
    (751, Some(0x40)),
    (754, Some(0x40)),
    (755, Some(0x49)),
    (757, Some(0x49)),
    (758, Some(0x49)),
    (759, Some(0x48)),
    (760, Some(0x4B)),
    (761, Some(0x4A)),
    (762, Some(0x4E)),
    (763, Some(0x4E)),
    (764, Some(0x50)),
    (765, Some(0x52)),
    (766, Some(0x54)),
    (767, Some(0x54)),
    (768, Some(0x58)),
    (769, Some(0x58)),
    (770, Some(0x57)),
    (771, Some(0x57)),
    (772, Some(0x57)),
    (773, Some(0x5C)),
    (774, Some(0x5C)),
    (775, Some(0x5E)),
    (776, Some(0x5E)),
];

const C_CHUNK_DATA: &[(i32, Option<i32>)] = &[
    (4, Some(0x21)),
    (5, Some(0x21)),
    (47, Some(0x21)),
    (107, Some(0x20)),
    (109, Some(0x20)),
    (110, Some(0x20)),
    (335, Some(0x20)),
    (338, Some(0x20)),
    (340, Some(0x20)),
    (393, Some(0x22)),
    (477, Some(0x21)),
    (573, Some(0x22)),
    (735, Some(0x21)),
    (751, Some(0x20)),
    (754, Some(0x20)),
    (755, Some(0x22)),
    (757, Some(0x22)),
    (758, Some(0x22)),
    (759, Some(0x1F)),
    (760, Some(0x21)),
    (761, Some(0x20)),
    (762, Some(0x24)),
    (763, Some(0x24)),
    (764, Some(0x25)),
    (765, Some(0x25)),
    (766, Some(0x27)),
    (767, Some(0x27)),
    (768, Some(0x28)),
    (769, Some(0x28)),
    (770, Some(0x27)),
    (771, Some(0x27)),
    (772, Some(0x27)),
    (773, Some(0x2C)),
    (774, Some(0x2C)),
    (775, Some(0x2D)),
    (776, Some(0x2D)),
];

const C_SET_DEFAULT_SPAWN_POSITION: &[(i32, Option<i32>)] = &[
    (4, Some(0x05)),
    (5, Some(0x05)),
    (47, Some(0x05)),
    (107, Some(0x43)),
    (109, Some(0x43)),
    (110, Some(0x43)),
    (335, Some(0x45)),
    (338, Some(0x46)),
    (340, Some(0x46)),
    (393, Some(0x49)),
    (477, Some(0x4D)),
    (573, Some(0x4E)),
    (735, Some(0x42)),
    (751, Some(0x42)),
    (754, Some(0x42)),
    (755, Some(0x4B)),
    (757, Some(0x4B)),
    (758, Some(0x4B)),
    (759, Some(0x4A)),
    (760, Some(0x4D)),
    (761, Some(0x4C)),
    (762, Some(0x50)),
    (763, Some(0x50)),
    (764, Some(0x52)),
    (765, Some(0x54)),
    (766, Some(0x56)),
    (767, Some(0x56)),
    (768, Some(0x5B)),
    (769, Some(0x5B)),
    (770, Some(0x5A)),
    (771, Some(0x5A)),
    (772, Some(0x5A)),
    (773, Some(0x5F)),
    (774, Some(0x5F)),
    (775, Some(0x61)),
    (776, Some(0x61)),
];

const C_SYNCHRONIZE_PLAYER_POSITION: &[(i32, Option<i32>)] = &[
    (4, Some(0x08)),
    (5, Some(0x08)),
    (47, Some(0x08)),
    (107, Some(0x2E)),
    (109, Some(0x2E)),
    (110, Some(0x2E)),
    (335, Some(0x2E)),
    (338, Some(0x2F)),
    (340, Some(0x2F)),
    (393, Some(0x32)),
    (477, Some(0x35)),
    (573, Some(0x36)),
    (735, Some(0x35)),
    (751, Some(0x34)),
    (754, Some(0x34)),
    (755, Some(0x38)),
    (757, Some(0x38)),
    (758, Some(0x38)),
    (759, Some(0x36)),
    (760, Some(0x39)),
    (761, Some(0x38)),
    (762, Some(0x3C)),
    (763, Some(0x3C)),
    (764, Some(0x3E)),
    (765, Some(0x3E)),
    (766, Some(0x40)),
    (767, Some(0x40)),
    (768, Some(0x42)),
    (769, Some(0x42)),
    (770, Some(0x41)),
    (771, Some(0x41)),
    (772, Some(0x41)),
    (773, Some(0x46)),
    (774, Some(0x46)),
    (775, Some(0x48)),
    (776, Some(0x48)),
];

#[test]
fn game_event_ids_match_every_protocol() {
    assert_ids::<CGameEvent>(C_GAME_EVENT);
}

#[test]
fn set_center_chunk_ids_match_every_protocol() {
    assert_ids::<CSetCenterChunk>(C_SET_CENTER_CHUNK);
}

#[test]
fn chunk_data_ids_match_every_protocol() {
    assert_ids::<CChunkData>(C_CHUNK_DATA);
}

#[test]
fn set_default_spawn_position_ids_match_every_protocol() {
    assert_ids::<CSetDefaultSpawnPosition>(C_SET_DEFAULT_SPAWN_POSITION);
}

#[test]
fn synchronize_player_position_ids_match_every_protocol() {
    assert_ids::<CSynchronizePlayerPosition>(C_SYNCHRONIZE_PLAYER_POSITION);
}
