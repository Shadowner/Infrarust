#![allow(clippy::unwrap_used, clippy::expect_used)]

use infrarust_protocol::packets::Packet;
use infrarust_protocol::registry::{PacketRegistry, build_default_registry};
use infrarust_protocol::version::ProtocolVersion;
use infrarust_protocol::{
    CChatMessageLegacy, CSystemChatMessage, SChatAcknowledgement, SChatCommand, SChatCommandSigned,
    SChatMessage, SChatSessionUpdate, SClientInformation, SConfigClientInformation, SKeepAlive,
};

const S_CHAT_MESSAGE: &[(i32, Option<i32>)] = &[
    (4, Some(0x01)),
    (5, Some(0x01)),
    (47, Some(0x01)),
    (107, Some(0x02)),
    (109, Some(0x02)),
    (110, Some(0x02)),
    (210, Some(0x02)),
    (315, Some(0x02)),
    (335, Some(0x03)),
    (338, Some(0x02)),
    (340, Some(0x02)),
    (393, Some(0x02)),
    (401, Some(0x02)),
    (404, Some(0x02)),
    (477, Some(0x03)),
    (498, Some(0x03)),
    (573, Some(0x03)),
    (578, Some(0x03)),
    (735, Some(0x03)),
    (736, Some(0x03)),
    (751, Some(0x03)),
    (754, Some(0x03)),
    (755, Some(0x03)),
    (756, Some(0x03)),
    (757, Some(0x03)),
    (758, Some(0x03)),
    (759, Some(0x04)),
    (760, Some(0x05)),
    (761, Some(0x05)),
    (762, Some(0x05)),
    (763, Some(0x05)),
    (764, Some(0x05)),
    (765, Some(0x05)),
    (766, Some(0x06)),
    (767, Some(0x06)),
    (768, Some(0x07)),
    (769, Some(0x07)),
    (770, Some(0x07)),
    (771, Some(0x08)),
    (772, Some(0x08)),
    (773, Some(0x08)),
    (774, Some(0x08)),
    (775, Some(0x09)),
    (776, Some(0x09)),
];

const S_CHAT_COMMAND: &[(i32, Option<i32>)] = &[
    (4, None),
    (5, None),
    (47, None),
    (107, None),
    (109, None),
    (110, None),
    (210, None),
    (315, None),
    (335, None),
    (338, None),
    (340, None),
    (393, None),
    (401, None),
    (404, None),
    (477, None),
    (498, None),
    (573, None),
    (578, None),
    (735, None),
    (736, None),
    (751, None),
    (754, None),
    (755, None),
    (756, None),
    (757, None),
    (758, None),
    (759, Some(0x03)),
    (760, Some(0x04)),
    (761, Some(0x04)),
    (762, Some(0x04)),
    (763, Some(0x04)),
    (764, Some(0x04)),
    (765, Some(0x04)),
    (766, Some(0x04)),
    (767, Some(0x04)),
    (768, Some(0x05)),
    (769, Some(0x05)),
    (770, Some(0x05)),
    (771, Some(0x06)),
    (772, Some(0x06)),
    (773, Some(0x06)),
    (774, Some(0x06)),
    (775, Some(0x07)),
    (776, Some(0x07)),
];

const S_CHAT_COMMAND_SIGNED: &[(i32, Option<i32>)] = &[
    (4, None),
    (5, None),
    (47, None),
    (107, None),
    (109, None),
    (110, None),
    (210, None),
    (315, None),
    (335, None),
    (338, None),
    (340, None),
    (393, None),
    (401, None),
    (404, None),
    (477, None),
    (498, None),
    (573, None),
    (578, None),
    (735, None),
    (736, None),
    (751, None),
    (754, None),
    (755, None),
    (756, None),
    (757, None),
    (758, None),
    (759, None),
    (760, None),
    (761, None),
    (762, None),
    (763, None),
    (764, None),
    (765, None),
    (766, Some(0x05)),
    (767, Some(0x05)),
    (768, Some(0x06)),
    (769, Some(0x06)),
    (770, Some(0x06)),
    (771, Some(0x07)),
    (772, Some(0x07)),
    (773, Some(0x07)),
    (774, Some(0x07)),
    (775, Some(0x08)),
    (776, Some(0x08)),
];

const S_CHAT_ACKNOWLEDGEMENT: &[(i32, Option<i32>)] = &[
    (4, None),
    (5, None),
    (47, None),
    (107, None),
    (109, None),
    (110, None),
    (210, None),
    (315, None),
    (335, None),
    (338, None),
    (340, None),
    (393, None),
    (401, None),
    (404, None),
    (477, None),
    (498, None),
    (573, None),
    (578, None),
    (735, None),
    (736, None),
    (751, None),
    (754, None),
    (755, None),
    (756, None),
    (757, None),
    (758, None),
    (759, None),
    (760, Some(0x03)),
    (761, Some(0x03)),
    (762, Some(0x03)),
    (763, Some(0x03)),
    (764, Some(0x03)),
    (765, Some(0x03)),
    (766, Some(0x03)),
    (767, Some(0x03)),
    (768, Some(0x04)),
    (769, Some(0x04)),
    (770, Some(0x04)),
    (771, Some(0x05)),
    (772, Some(0x05)),
    (773, Some(0x05)),
    (774, Some(0x05)),
    (775, Some(0x06)),
    (776, Some(0x06)),
];

const S_CHAT_SESSION_UPDATE: &[(i32, Option<i32>)] = &[
    (4, None),
    (5, None),
    (47, None),
    (107, None),
    (109, None),
    (110, None),
    (210, None),
    (315, None),
    (335, None),
    (338, None),
    (340, None),
    (393, None),
    (401, None),
    (404, None),
    (477, None),
    (498, None),
    (573, None),
    (578, None),
    (735, None),
    (736, None),
    (751, None),
    (754, None),
    (755, None),
    (756, None),
    (757, None),
    (758, None),
    (759, None),
    (760, None),
    (761, Some(0x20)),
    (762, Some(0x06)),
    (763, Some(0x06)),
    (764, Some(0x06)),
    (765, Some(0x06)),
    (766, Some(0x07)),
    (767, Some(0x07)),
    (768, Some(0x08)),
    (769, Some(0x08)),
    (770, Some(0x08)),
    (771, Some(0x09)),
    (772, Some(0x09)),
    (773, Some(0x09)),
    (774, Some(0x09)),
    (775, Some(0x0A)),
    (776, Some(0x0A)),
];

const S_CLIENT_INFORMATION: &[(i32, Option<i32>)] = &[
    (4, Some(0x15)),
    (5, Some(0x15)),
    (47, Some(0x15)),
    (107, Some(0x04)),
    (109, Some(0x04)),
    (110, Some(0x04)),
    (210, Some(0x04)),
    (315, Some(0x04)),
    (335, Some(0x05)),
    (338, Some(0x04)),
    (340, Some(0x04)),
    (393, Some(0x04)),
    (401, Some(0x04)),
    (404, Some(0x04)),
    (477, Some(0x05)),
    (498, Some(0x05)),
    (573, Some(0x05)),
    (578, Some(0x05)),
    (735, Some(0x05)),
    (736, Some(0x05)),
    (751, Some(0x05)),
    (754, Some(0x05)),
    (755, Some(0x05)),
    (756, Some(0x05)),
    (757, Some(0x05)),
    (758, Some(0x05)),
    (759, Some(0x07)),
    (760, Some(0x08)),
    (761, Some(0x07)),
    (762, Some(0x08)),
    (763, Some(0x08)),
    (764, Some(0x09)),
    (765, Some(0x09)),
    (766, Some(0x0A)),
    (767, Some(0x0A)),
    (768, Some(0x0C)),
    (769, Some(0x0C)),
    (770, Some(0x0C)),
    (771, Some(0x0D)),
    (772, Some(0x0D)),
    (773, Some(0x0D)),
    (774, Some(0x0D)),
    (775, Some(0x0E)),
    (776, Some(0x0E)),
];

const S_CONFIG_CLIENT_INFORMATION: &[(i32, Option<i32>)] = &[
    (4, None),
    (5, None),
    (47, None),
    (107, None),
    (109, None),
    (110, None),
    (210, None),
    (315, None),
    (335, None),
    (338, None),
    (340, None),
    (393, None),
    (401, None),
    (404, None),
    (477, None),
    (498, None),
    (573, None),
    (578, None),
    (735, None),
    (736, None),
    (751, None),
    (754, None),
    (755, None),
    (756, None),
    (757, None),
    (758, None),
    (759, None),
    (760, None),
    (761, None),
    (762, None),
    (763, None),
    (764, Some(0x00)),
    (765, Some(0x00)),
    (766, Some(0x00)),
    (767, Some(0x00)),
    (768, Some(0x00)),
    (769, Some(0x00)),
    (770, Some(0x00)),
    (771, Some(0x00)),
    (772, Some(0x00)),
    (773, Some(0x00)),
    (774, Some(0x00)),
    (775, Some(0x00)),
    (776, Some(0x00)),
];

const S_KEEP_ALIVE: &[(i32, Option<i32>)] = &[
    (4, Some(0x00)),
    (5, Some(0x00)),
    (47, Some(0x00)),
    (107, Some(0x0B)),
    (109, Some(0x0B)),
    (110, Some(0x0B)),
    (210, Some(0x0B)),
    (315, Some(0x0B)),
    (335, Some(0x0C)),
    (338, Some(0x0B)),
    (340, Some(0x0B)),
    (393, Some(0x0E)),
    (401, Some(0x0E)),
    (404, Some(0x0E)),
    (477, Some(0x0F)),
    (498, Some(0x0F)),
    (573, Some(0x0F)),
    (578, Some(0x0F)),
    (735, Some(0x10)),
    (736, Some(0x10)),
    (751, Some(0x10)),
    (754, Some(0x10)),
    (755, Some(0x0F)),
    (756, Some(0x0F)),
    (757, Some(0x0F)),
    (758, Some(0x0F)),
    (759, Some(0x11)),
    (760, Some(0x12)),
    (761, Some(0x11)),
    (762, Some(0x12)),
    (763, Some(0x12)),
    (764, Some(0x14)),
    (765, Some(0x15)),
    (766, Some(0x18)),
    (767, Some(0x18)),
    (768, Some(0x1A)),
    (769, Some(0x1A)),
    (770, Some(0x1A)),
    (771, Some(0x1B)),
    (772, Some(0x1B)),
    (773, Some(0x1B)),
    (774, Some(0x1B)),
    (775, Some(0x1C)),
    (776, Some(0x1C)),
];

const C_SYSTEM_CHAT_MESSAGE: &[(i32, Option<i32>)] = &[
    (4, None),
    (5, None),
    (47, None),
    (107, None),
    (109, None),
    (110, None),
    (210, None),
    (315, None),
    (335, None),
    (338, None),
    (340, None),
    (393, None),
    (401, None),
    (404, None),
    (477, None),
    (498, None),
    (573, None),
    (578, None),
    (735, None),
    (736, None),
    (751, None),
    (754, None),
    (755, None),
    (756, None),
    (757, None),
    (758, None),
    (759, Some(0x5F)),
    (760, Some(0x62)),
    (761, Some(0x60)),
    (762, Some(0x64)),
    (763, Some(0x64)),
    (764, Some(0x67)),
    (765, Some(0x69)),
    (766, Some(0x6C)),
    (767, Some(0x6C)),
    (768, Some(0x73)),
    (769, Some(0x73)),
    (770, Some(0x72)),
    (771, Some(0x72)),
    (772, Some(0x72)),
    (773, Some(0x77)),
    (774, Some(0x77)),
    (775, Some(0x79)),
    (776, Some(0x79)),
];

const C_CHAT_MESSAGE_LEGACY: &[(i32, Option<i32>)] = &[
    (4, Some(0x02)),
    (5, Some(0x02)),
    (47, Some(0x02)),
    (107, Some(0x0F)),
    (109, Some(0x0F)),
    (110, Some(0x0F)),
    (210, Some(0x0F)),
    (315, Some(0x0F)),
    (335, Some(0x0F)),
    (338, Some(0x0F)),
    (340, Some(0x0F)),
    (393, Some(0x0E)),
    (401, Some(0x0E)),
    (404, Some(0x0E)),
    (477, Some(0x0E)),
    (498, Some(0x0E)),
    (573, Some(0x0F)),
    (578, Some(0x0F)),
    (735, Some(0x0E)),
    (736, Some(0x0E)),
    (751, Some(0x0E)),
    (754, Some(0x0E)),
    (755, Some(0x0F)),
    (756, Some(0x0F)),
    (757, Some(0x0F)),
    (758, Some(0x0F)),
    (759, None),
    (760, None),
    (761, None),
    (762, None),
    (763, None),
    (764, None),
    (765, None),
    (766, None),
    (767, None),
    (768, None),
    (769, None),
    (770, None),
    (771, None),
    (772, None),
    (773, None),
    (774, None),
    (775, None),
    (776, None),
];
fn assert_ids<P: Packet>(expected: &[(i32, Option<i32>)]) {
    let registry = build_default_registry();
    let mismatches: Vec<String> = expected
        .iter()
        .filter_map(|&(protocol, id)| {
            let actual = registry.get_packet_id::<P>(ProtocolVersion(protocol));
            (actual != id)
                .then(|| format!("protocol {protocol}: expected {id:02X?}, got {actual:02X?}"))
        })
        .collect();
    assert!(
        mismatches.is_empty(),
        "{} id mismatches:\n{}",
        P::NAME,
        mismatches.join("\n")
    );
}

fn assert_decodable<P: Packet>(registry: &PacketRegistry, expected: &[(i32, Option<i32>)]) {
    for &(protocol, id) in expected {
        if let Some(id) = id {
            assert!(
                registry.has_decoder(P::STATE, P::DIRECTION, ProtocolVersion(protocol), id),
                "{} has no decoder on 0x{id:02X} at protocol {protocol}",
                P::NAME
            );
        }
    }
}

#[test]
fn chat_message_ids_match_every_protocol() {
    assert_ids::<SChatMessage>(S_CHAT_MESSAGE);
}

#[test]
fn chat_command_ids_match_every_protocol() {
    assert_ids::<SChatCommand>(S_CHAT_COMMAND);
}

#[test]
fn signed_chat_command_ids_match_every_protocol() {
    assert_ids::<SChatCommandSigned>(S_CHAT_COMMAND_SIGNED);
}

#[test]
fn chat_acknowledgement_ids_match_every_protocol() {
    assert_ids::<SChatAcknowledgement>(S_CHAT_ACKNOWLEDGEMENT);
}

#[test]
fn chat_session_update_ids_match_every_protocol() {
    assert_ids::<SChatSessionUpdate>(S_CHAT_SESSION_UPDATE);
}

#[test]
fn client_information_ids_match_every_protocol() {
    assert_ids::<SClientInformation>(S_CLIENT_INFORMATION);
}

#[test]
fn config_client_information_ids_match_every_protocol() {
    assert_ids::<SConfigClientInformation>(S_CONFIG_CLIENT_INFORMATION);
}

#[test]
fn serverbound_keep_alive_ids_match_every_protocol() {
    assert_ids::<SKeepAlive>(S_KEEP_ALIVE);
}

#[test]
fn system_chat_ids_match_every_protocol() {
    assert_ids::<CSystemChatMessage>(C_SYSTEM_CHAT_MESSAGE);
}

#[test]
fn legacy_chat_ids_match_every_protocol() {
    assert_ids::<CChatMessageLegacy>(C_CHAT_MESSAGE_LEGACY);
}

#[test]
fn chat_and_client_information_packets_decode_through_the_registry() {
    let registry = build_default_registry();
    assert_decodable::<SChatMessage>(&registry, S_CHAT_MESSAGE);
    assert_decodable::<SChatCommand>(&registry, S_CHAT_COMMAND);
    assert_decodable::<SChatCommandSigned>(&registry, S_CHAT_COMMAND_SIGNED);
    assert_decodable::<SChatAcknowledgement>(&registry, S_CHAT_ACKNOWLEDGEMENT);
    assert_decodable::<SChatSessionUpdate>(&registry, S_CHAT_SESSION_UPDATE);
    assert_decodable::<SClientInformation>(&registry, S_CLIENT_INFORMATION);
    assert_decodable::<SConfigClientInformation>(&registry, S_CONFIG_CLIENT_INFORMATION);
    assert_decodable::<CSystemChatMessage>(&registry, C_SYSTEM_CHAT_MESSAGE);
    assert_decodable::<CChatMessageLegacy>(&registry, C_CHAT_MESSAGE_LEGACY);
}

#[test]
fn protocol_759_uses_the_1_19_ids_not_the_1_19_1_ids() {
    let registry = build_default_registry();
    let v1_19 = ProtocolVersion::V1_19;
    let v1_19_1 = ProtocolVersion::V1_19_1;

    assert_eq!(registry.get_packet_id::<SChatCommand>(v1_19), Some(0x03));
    assert_eq!(registry.get_packet_id::<SChatMessage>(v1_19), Some(0x04));
    assert_eq!(registry.get_packet_id::<SChatAcknowledgement>(v1_19), None);
    assert_eq!(
        registry.get_packet_id::<SClientInformation>(v1_19),
        Some(0x07)
    );
    assert_eq!(registry.get_packet_id::<SKeepAlive>(v1_19), Some(0x11));
    assert_eq!(
        registry.get_packet_id::<CSystemChatMessage>(v1_19),
        Some(0x5F)
    );

    assert_eq!(
        registry.get_packet_id::<SChatAcknowledgement>(v1_19_1),
        Some(0x03)
    );
    assert_eq!(registry.get_packet_id::<SChatCommand>(v1_19_1), Some(0x04));
    assert_eq!(registry.get_packet_id::<SChatMessage>(v1_19_1), Some(0x05));
    assert_eq!(
        registry.get_packet_id::<SClientInformation>(v1_19_1),
        Some(0x08)
    );
    assert_eq!(registry.get_packet_id::<SKeepAlive>(v1_19_1), Some(0x12));
    assert_eq!(
        registry.get_packet_id::<CSystemChatMessage>(v1_19_1),
        Some(0x62)
    );
}
