#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bytes::Bytes;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry, build_default_registry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use infrarust_protocol::{
    ArgumentSignature, CChatMessageLegacy, CSystemChatMessage, ClientInformation, LastSeenMessages,
    PreviousMessage, PreviousMessages, SChatAcknowledgement, SChatCommand, SChatCommandSigned,
    SChatMessage, SChatSessionUpdate, SClientInformation, SConfigClientInformation,
};
use proptest::prelude::*;

fn versions_from(first: ProtocolVersion) -> impl Iterator<Item = ProtocolVersion> {
    ProtocolVersion::SUPPORTED
        .iter()
        .copied()
        .filter(move |v| *v >= first)
}

fn round_trip<P: Packet>(packet: &P, version: ProtocolVersion) -> P {
    let mut buf = Vec::new();
    packet.encode(&mut buf, version).unwrap();
    let mut r = buf.as_slice();
    let decoded = P::decode(&mut r, version).unwrap();
    assert!(
        r.is_empty(),
        "{} left {} bytes at {version}",
        P::NAME,
        r.len()
    );
    decoded
}

fn chat_message_as_seen_by(packet: &SChatMessage, version: ProtocolVersion) -> SChatMessage {
    if version < ProtocolVersion::V1_19 {
        return SChatMessage {
            message: packet.message.clone(),
            ..SChatMessage::default()
        };
    }
    if version < ProtocolVersion::V1_19_3 {
        return SChatMessage {
            last_seen: LastSeenMessages::default(),
            previous_messages: if version < ProtocolVersion::V1_19_1 {
                PreviousMessages::default()
            } else {
                packet.previous_messages.clone()
            },
            ..packet.clone()
        };
    }
    SChatMessage {
        signed_preview: false,
        previous_messages: PreviousMessages::default(),
        last_seen: last_seen_as_seen_by(packet.last_seen, version),
        ..packet.clone()
    }
}

fn chat_command_as_seen_by(packet: &SChatCommand, version: ProtocolVersion) -> SChatCommand {
    if version >= ProtocolVersion::V1_20_5 {
        return SChatCommand {
            command: packet.command.clone(),
            ..SChatCommand::default()
        };
    }
    if version < ProtocolVersion::V1_19_3 {
        return SChatCommand {
            last_seen: LastSeenMessages::default(),
            previous_messages: if version < ProtocolVersion::V1_19_1 {
                PreviousMessages::default()
            } else {
                packet.previous_messages.clone()
            },
            ..packet.clone()
        };
    }
    SChatCommand {
        signed_preview: false,
        previous_messages: PreviousMessages::default(),
        last_seen: last_seen_as_seen_by(packet.last_seen, version),
        ..packet.clone()
    }
}

fn last_seen_as_seen_by(last_seen: LastSeenMessages, version: ProtocolVersion) -> LastSeenMessages {
    if version < ProtocolVersion::V1_21_5 {
        LastSeenMessages {
            checksum: 0,
            ..last_seen
        }
    } else {
        last_seen
    }
}

fn client_information_as_seen_by(
    information: &ClientInformation,
    version: ProtocolVersion,
) -> ClientInformation {
    ClientInformation {
        difficulty: if version < ProtocolVersion::V1_8 {
            information.difficulty
        } else {
            0
        },
        main_hand: if version < ProtocolVersion::V1_9 {
            1
        } else {
            information.main_hand
        },
        text_filtering: version >= ProtocolVersion::V1_17 && information.text_filtering,
        allow_server_listings: version < ProtocolVersion::V1_18
            || information.allow_server_listings,
        particle_status: if version < ProtocolVersion::V1_21_2 {
            0
        } else {
            information.particle_status
        },
        ..information.clone()
    }
}

fn signature() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 256)
}

fn previous_message() -> impl Strategy<Value = PreviousMessage> {
    (
        any::<u128>(),
        proptest::collection::vec(any::<u8>(), 0..300),
    )
        .prop_map(|(sender, signature)| PreviousMessage {
            sender: uuid::Uuid::from_u128(sender),
            signature,
        })
}

fn previous_messages() -> impl Strategy<Value = PreviousMessages> {
    (
        proptest::collection::vec(previous_message(), 0..6),
        proptest::option::of(previous_message()),
    )
        .prop_map(|(seen, last_received)| PreviousMessages {
            seen,
            last_received,
        })
}

fn last_seen() -> impl Strategy<Value = LastSeenMessages> {
    (any::<i32>(), any::<[u8; 3]>(), any::<u8>()).prop_map(|(offset, acknowledged, checksum)| {
        LastSeenMessages {
            offset,
            acknowledged,
            checksum,
        }
    })
}

fn argument_signatures() -> impl Strategy<Value = Vec<ArgumentSignature>> {
    proptest::collection::vec(
        ("[a-z_]{0,16}", signature())
            .prop_map(|(name, signature)| ArgumentSignature { name, signature }),
        0..8,
    )
}

fn chat_message() -> impl Strategy<Value = SChatMessage> {
    (
        "\\PC{0,256}",
        any::<i64>(),
        any::<i64>(),
        proptest::option::of(signature()),
        any::<bool>(),
        previous_messages(),
        last_seen(),
    )
        .prop_map(
            |(
                message,
                timestamp,
                salt,
                signature,
                signed_preview,
                previous_messages,
                last_seen,
            )| {
                SChatMessage {
                    message,
                    timestamp,
                    salt,
                    signature,
                    signed_preview,
                    previous_messages,
                    last_seen,
                }
            },
        )
}

fn chat_command() -> impl Strategy<Value = SChatCommand> {
    (
        "\\PC{0,256}",
        any::<i64>(),
        any::<i64>(),
        argument_signatures(),
        any::<bool>(),
        previous_messages(),
        last_seen(),
    )
        .prop_map(
            |(
                command,
                timestamp,
                salt,
                argument_signatures,
                signed_preview,
                previous_messages,
                last_seen,
            )| SChatCommand {
                command,
                timestamp,
                salt,
                argument_signatures,
                signed_preview,
                previous_messages,
                last_seen,
            },
        )
}

fn client_information() -> impl Strategy<Value = ClientInformation> {
    (
        "[a-z_]{0,16}",
        any::<i8>(),
        any::<i8>(),
        any::<bool>(),
        any::<u8>(),
        any::<u8>(),
        any::<i32>(),
        any::<bool>(),
        any::<bool>(),
        any::<i32>(),
    )
        .prop_map(
            |(
                locale,
                view_distance,
                chat_mode,
                chat_colors,
                difficulty,
                displayed_skin_parts,
                main_hand,
                text_filtering,
                allow_server_listings,
                particle_status,
            )| ClientInformation {
                locale,
                view_distance,
                chat_mode: i32::from(chat_mode),
                chat_colors,
                difficulty,
                displayed_skin_parts,
                main_hand,
                text_filtering,
                allow_server_listings,
                particle_status,
            },
        )
}

fn decode_everything(bytes: &[u8], version: ProtocolVersion) {
    let _ = SChatMessage::decode(&mut &bytes[..], version);
    let _ = SChatCommand::decode(&mut &bytes[..], version);
    let _ = SChatCommandSigned::decode(&mut &bytes[..], version);
    let _ = SChatAcknowledgement::decode(&mut &bytes[..], version);
    let _ = SChatSessionUpdate::decode(&mut &bytes[..], version);
    let _ = SClientInformation::decode(&mut &bytes[..], version);
    let _ = SConfigClientInformation::decode(&mut &bytes[..], version);
    let _ = CSystemChatMessage::decode(&mut &bytes[..], version);
    let _ = CChatMessageLegacy::decode(&mut &bytes[..], version);
}

fn decode_every_frame(registry: &PacketRegistry, bytes: &[u8], version: ProtocolVersion) {
    let payload = Bytes::copy_from_slice(bytes);
    for (state, direction) in [
        (ConnectionState::Play, Direction::Serverbound),
        (ConnectionState::Play, Direction::Clientbound),
        (ConnectionState::Config, Direction::Serverbound),
    ] {
        for id in 0..0x80 {
            let frame = PacketFrame::new(id, payload.clone());
            let _ = registry.decode_frame(&frame, state, direction, version);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn chat_message_round_trips_in_every_format(packet in chat_message()) {
        for version in ProtocolVersion::SUPPORTED.iter().copied() {
            prop_assert_eq!(
                round_trip(&packet, version),
                chat_message_as_seen_by(&packet, version),
                "{}", version
            );
        }
    }

    #[test]
    fn chat_command_round_trips_in_every_format(packet in chat_command()) {
        for version in versions_from(ProtocolVersion::V1_19) {
            prop_assert_eq!(
                round_trip(&packet, version),
                chat_command_as_seen_by(&packet, version),
                "{}", version
            );
        }
    }

    #[test]
    fn signed_chat_command_round_trips(
        command in "\\PC{0,256}",
        timestamp in any::<i64>(),
        salt in any::<i64>(),
        argument_signatures in argument_signatures(),
        last_seen in last_seen(),
    ) {
        let packet = SChatCommandSigned { command, timestamp, salt, argument_signatures, last_seen };
        for version in versions_from(ProtocolVersion::V1_20_5) {
            prop_assert_eq!(
                round_trip(&packet, version),
                SChatCommandSigned {
                    last_seen: last_seen_as_seen_by(packet.last_seen, version),
                    ..packet.clone()
                },
                "{}", version
            );
        }
    }

    #[test]
    fn chat_acknowledgement_round_trips(offset in any::<i32>(), previous in previous_messages()) {
        let packet = SChatAcknowledgement { offset, previous_messages: previous };
        for version in versions_from(ProtocolVersion::V1_19_1) {
            let expected = if version < ProtocolVersion::V1_19_3 {
                SChatAcknowledgement { offset: 0, ..packet.clone() }
            } else {
                SChatAcknowledgement { previous_messages: PreviousMessages::default(), ..packet.clone() }
            };
            prop_assert_eq!(round_trip(&packet, version), expected, "{}", version);
        }
    }

    #[test]
    fn client_information_round_trips_in_every_format(information in client_information()) {
        let play = SClientInformation { information: information.clone() };
        for version in ProtocolVersion::SUPPORTED.iter().copied() {
            prop_assert_eq!(
                round_trip(&play, version).information,
                client_information_as_seen_by(&information, version),
                "{}", version
            );
        }
        let config = SConfigClientInformation { information: information.clone() };
        for version in versions_from(ProtocolVersion::V1_20_2) {
            prop_assert_eq!(
                round_trip(&config, version).information,
                client_information_as_seen_by(&information, version),
                "{}", version
            );
        }
    }

    #[test]
    fn decoding_garbage_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..700)) {
        for version in ProtocolVersion::SUPPORTED.iter().copied() {
            decode_everything(&bytes, version);
        }
    }

    #[test]
    fn decoding_corrupted_chat_never_panics(
        packet in chat_message(),
        command in chat_command(),
        flips in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8),
        cut in any::<prop::sample::Index>(),
    ) {
        for version in versions_from(ProtocolVersion::V1_19) {
            for bytes in [encode(&packet, version), encode(&command, version)] {
                let mut corrupted = bytes.clone();
                for (index, value) in &flips {
                    let i = index.index(corrupted.len());
                    corrupted[i] = *value;
                }
                decode_everything(&corrupted, version);
                decode_everything(&bytes[..cut.index(bytes.len() + 1)], version);
            }
        }
    }
}

fn encode<P: Packet>(packet: &P, version: ProtocolVersion) -> Vec<u8> {
    let mut buf = Vec::new();
    packet.encode(&mut buf, version).unwrap();
    buf
}

#[test]
fn registry_never_panics_on_garbage_frames() {
    let registry = build_default_registry();
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    for _ in 0..64 {
        let len = usize::try_from(state % 320).unwrap();
        let bytes: Vec<u8> = (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state.to_le_bytes()[0]
            })
            .collect();
        for version in ProtocolVersion::SUPPORTED.iter().copied() {
            decode_every_frame(&registry, &bytes, version);
        }
    }
}

fn typed<P: Packet>(
    registry: &PacketRegistry,
    packet: &P,
    version: ProtocolVersion,
) -> Box<dyn infrarust_protocol::ErasedPacket> {
    let id = registry.get_packet_id::<P>(version).unwrap();
    let frame = PacketFrame::new(id, Bytes::from(encode(packet, version)));
    match registry
        .decode_frame(&frame, P::STATE, P::DIRECTION, version)
        .unwrap()
    {
        DecodedPacket::Typed {
            id: decoded_id,
            packet,
        } => {
            assert_eq!(decoded_id, id);
            packet
        }
        DecodedPacket::Opaque { .. } => panic!("{} decoded as opaque at {version}", P::NAME),
    }
}

#[test]
fn registry_decodes_chat_packets_at_every_format_boundary() {
    let registry = build_default_registry();
    let message = SChatMessage {
        message: "hello".to_string(),
        timestamp: 1_700_000_000_000,
        ..SChatMessage::default()
    };
    let command = SChatCommand {
        command: "spawn".to_string(),
        timestamp: 1_700_000_000_000,
        ..SChatCommand::default()
    };

    for protocol in [
        47, 340, 758, 759, 760, 761, 762, 764, 765, 766, 768, 770, 771, 774, 775, 776,
    ] {
        let version = ProtocolVersion(protocol);
        let decoded = typed(&registry, &message, version);
        let decoded = decoded.as_any().downcast_ref::<SChatMessage>().unwrap();
        assert_eq!(decoded.message, "hello", "{version}");

        if version >= ProtocolVersion::V1_19 {
            let decoded = typed(&registry, &command, version);
            let decoded = decoded.as_any().downcast_ref::<SChatCommand>().unwrap();
            assert_eq!(decoded.command, "spawn", "{version}");
        }
        if version >= ProtocolVersion::V1_19_1 {
            let ack = SChatAcknowledgement {
                offset: 3,
                ..SChatAcknowledgement::default()
            };
            let decoded = typed(&registry, &ack, version);
            assert!(decoded.as_any().is::<SChatAcknowledgement>(), "{version}");
        }
        if version >= ProtocolVersion::V1_20_5 {
            let signed = SChatCommandSigned {
                command: "msg a b".to_string(),
                ..SChatCommandSigned::default()
            };
            let decoded = typed(&registry, &signed, version);
            assert!(decoded.as_any().is::<SChatCommandSigned>(), "{version}");
        }
    }
}

#[test]
fn registry_decodes_system_chat_and_client_information() {
    let registry = build_default_registry();
    let information = ClientInformation {
        locale: "en_us".to_string(),
        view_distance: 10,
        chat_mode: 0,
        chat_colors: true,
        difficulty: 0,
        displayed_skin_parts: 0x7F,
        main_hand: 1,
        text_filtering: false,
        allow_server_listings: true,
        particle_status: 0,
    };

    for protocol in [
        4, 47, 107, 340, 758, 759, 760, 761, 764, 766, 768, 771, 775, 776,
    ] {
        let version = ProtocolVersion(protocol);
        let packet = SClientInformation {
            information: information.clone(),
        };
        let decoded = typed(&registry, &packet, version);
        let decoded = decoded
            .as_any()
            .downcast_ref::<SClientInformation>()
            .unwrap();
        assert_eq!(decoded.information, information, "{version}");

        if version >= ProtocolVersion::V1_19 {
            let chat = if version < ProtocolVersion::V1_20_3 {
                CSystemChatMessage::from_json(r#"{"text":"hi"}"#, true)
            } else {
                CSystemChatMessage::from_nbt(vec![0x08, 0x00, 0x02, b'h', b'i'], true)
            };
            let decoded = typed(&registry, &chat, version);
            let decoded = decoded
                .as_any()
                .downcast_ref::<CSystemChatMessage>()
                .unwrap();
            assert_eq!(decoded.content, chat.content, "{version}");
            assert!(decoded.overlay);
        } else {
            let legacy = CChatMessageLegacy {
                content: r#"{"text":"hi"}"#.to_string(),
                position: 1,
            };
            let decoded = typed(&registry, &legacy, version);
            assert!(decoded.as_any().is::<CChatMessageLegacy>(), "{version}");
        }

        if version >= ProtocolVersion::V1_20_2 {
            let packet = SConfigClientInformation {
                information: information.clone(),
            };
            let decoded = typed(&registry, &packet, version);
            assert!(
                decoded.as_any().is::<SConfigClientInformation>(),
                "{version}"
            );
        }
    }
}
