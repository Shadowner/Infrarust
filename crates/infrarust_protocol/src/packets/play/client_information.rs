use std::io::Write;

use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::{ProtocolError, ProtocolResult};
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

const LOCALE_MAX_CHARS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInformation {
    pub locale: String,
    pub view_distance: i8,
    pub chat_mode: i32,
    pub chat_colors: bool,
    pub difficulty: u8,
    pub displayed_skin_parts: u8,
    pub main_hand: i32,
    pub text_filtering: bool,
    pub allow_server_listings: bool,
    pub particle_status: i32,
}

pub(crate) fn decode_client_information(
    r: &mut &[u8],
    version: ProtocolVersion,
) -> ProtocolResult<ClientInformation> {
    let locale = r.read_string_bounded(LOCALE_MAX_CHARS)?;
    let view_distance = r.read_i8()?;
    let chat_mode = if version.no_less_than(ProtocolVersion::V1_9) {
        r.read_var_int()?.0
    } else {
        i32::from(r.read_i8()?)
    };
    let chat_colors = r.read_bool()?;
    let difficulty = if version.less_than(ProtocolVersion::V1_8) {
        r.read_u8()?
    } else {
        0
    };
    let displayed_skin_parts = r.read_u8()?;
    let main_hand = if version.no_less_than(ProtocolVersion::V1_9) {
        r.read_var_int()?.0
    } else {
        1
    };
    let text_filtering = version.no_less_than(ProtocolVersion::V1_17) && r.read_bool()?;
    let allow_server_listings = if version.no_less_than(ProtocolVersion::V1_18) {
        r.read_bool()?
    } else {
        true
    };
    let particle_status = if version.no_less_than(ProtocolVersion::V1_21_2) {
        r.read_var_int()?.0
    } else {
        0
    };
    Ok(ClientInformation {
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
    })
}

pub(crate) fn encode_client_information(
    mut w: &mut (impl Write + ?Sized),
    information: &ClientInformation,
    version: ProtocolVersion,
) -> ProtocolResult<()> {
    w.write_string(&information.locale)?;
    w.write_i8(information.view_distance)?;
    if version.no_less_than(ProtocolVersion::V1_9) {
        w.write_var_int(&VarInt(information.chat_mode))?;
    } else {
        let chat_mode = i8::try_from(information.chat_mode).map_err(|_| {
            ProtocolError::invalid(format!(
                "chat mode {} does not fit the byte used before 1.9",
                information.chat_mode
            ))
        })?;
        w.write_i8(chat_mode)?;
    }
    w.write_bool(information.chat_colors)?;
    if version.less_than(ProtocolVersion::V1_8) {
        w.write_u8(information.difficulty)?;
    }
    w.write_u8(information.displayed_skin_parts)?;
    if version.no_less_than(ProtocolVersion::V1_9) {
        w.write_var_int(&VarInt(information.main_hand))?;
    }
    if version.no_less_than(ProtocolVersion::V1_17) {
        w.write_bool(information.text_filtering)?;
    }
    if version.no_less_than(ProtocolVersion::V1_18) {
        w.write_bool(information.allow_server_listings)?;
    }
    if version.no_less_than(ProtocolVersion::V1_21_2) {
        w.write_var_int(&VarInt(information.particle_status))?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SClientInformation {
    pub information: ClientInformation,
}

impl Packet for SClientInformation {
    const NAME: &'static str = "SClientInformation";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Serverbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_7_2  => 0x15,
        V1_9    => 0x04,
        V1_12   => 0x05,
        V1_12_1 => 0x04,
        V1_14   => 0x05,
        V1_19   => 0x07,
        V1_19_1 => 0x08,
        V1_19_3 => 0x07,
        V1_19_4 => 0x08,
        V1_20_2 => 0x09,
        V1_20_5 => 0x0A,
        V1_21_2 => 0x0C,
        V1_21_6 => 0x0D,
        V26_1   => 0x0E,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let information = decode_client_information(r, version)?;
        Ok(Self { information })
    }

    fn encode(
        &self,
        w: &mut (impl Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        encode_client_information(w, &self.information, version)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn information() -> ClientInformation {
        ClientInformation {
            locale: "en_gb".to_string(),
            view_distance: 12,
            chat_mode: 1,
            chat_colors: true,
            difficulty: 2,
            displayed_skin_parts: 0x7F,
            main_hand: 0,
            text_filtering: true,
            allow_server_listings: false,
            particle_status: 2,
        }
    }

    const LOCALE: &[u8] = &[0x05, b'e', b'n', b'_', b'g', b'b'];

    fn encoded(version: ProtocolVersion) -> Vec<u8> {
        let mut buf = Vec::new();
        SClientInformation {
            information: information(),
        }
        .encode(&mut buf, version)
        .unwrap();
        buf
    }

    fn decoded(bytes: &[u8], version: ProtocolVersion) -> ClientInformation {
        let mut r = bytes;
        let packet = SClientInformation::decode(&mut r, version).unwrap();
        assert!(r.is_empty(), "{} unread bytes", r.len());
        packet.information
    }

    #[test]
    fn test_1_7_carries_difficulty_and_the_cape_flag() {
        let expected = [LOCALE, &[0x0C, 0x01, 0x01, 0x02, 0x7F]].concat();
        for version in [ProtocolVersion::V1_7_2, ProtocolVersion::V1_7_6] {
            assert_eq!(encoded(version), expected, "{version}");
            assert_eq!(
                decoded(&expected, version),
                ClientInformation {
                    main_hand: 1,
                    text_filtering: false,
                    allow_server_listings: true,
                    particle_status: 0,
                    ..information()
                }
            );
        }
    }

    #[test]
    fn test_1_8_drops_difficulty_for_skin_parts() {
        let expected = [LOCALE, &[0x0C, 0x01, 0x01, 0x7F]].concat();
        assert_eq!(encoded(ProtocolVersion::V1_8), expected);
        assert_eq!(
            decoded(&expected, ProtocolVersion::V1_8),
            ClientInformation {
                difficulty: 0,
                main_hand: 1,
                text_filtering: false,
                allow_server_listings: true,
                particle_status: 0,
                ..information()
            }
        );
    }

    #[test]
    fn test_1_9_adds_main_hand() {
        let expected = [LOCALE, &[0x0C, 0x01, 0x01, 0x7F, 0x00]].concat();
        for version in [ProtocolVersion::V1_9, ProtocolVersion::V1_16_4] {
            assert_eq!(encoded(version), expected, "{version}");
            assert_eq!(
                decoded(&expected, version),
                ClientInformation {
                    difficulty: 0,
                    text_filtering: false,
                    allow_server_listings: true,
                    particle_status: 0,
                    ..information()
                }
            );
        }
    }

    #[test]
    fn test_1_17_adds_text_filtering() {
        let expected = [LOCALE, &[0x0C, 0x01, 0x01, 0x7F, 0x00, 0x01]].concat();
        assert_eq!(encoded(ProtocolVersion::V1_17), expected);
        assert_eq!(
            decoded(&expected, ProtocolVersion::V1_17),
            ClientInformation {
                difficulty: 0,
                allow_server_listings: true,
                particle_status: 0,
                ..information()
            }
        );
    }

    #[test]
    fn test_1_18_adds_server_listing_opt_out() {
        let expected = [LOCALE, &[0x0C, 0x01, 0x01, 0x7F, 0x00, 0x01, 0x00]].concat();
        for version in [
            ProtocolVersion::V1_18,
            ProtocolVersion::V1_20_2,
            ProtocolVersion::V1_21,
        ] {
            assert_eq!(encoded(version), expected, "{version}");
            assert_eq!(
                decoded(&expected, version),
                ClientInformation {
                    difficulty: 0,
                    particle_status: 0,
                    ..information()
                }
            );
        }
    }

    #[test]
    fn test_1_21_2_adds_particle_status() {
        let expected = [LOCALE, &[0x0C, 0x01, 0x01, 0x7F, 0x00, 0x01, 0x00, 0x02]].concat();
        for version in [
            ProtocolVersion::V1_21_2,
            ProtocolVersion::V1_21_11,
            ProtocolVersion::V26_2,
        ] {
            assert_eq!(encoded(version), expected, "{version}");
            assert_eq!(
                decoded(&expected, version),
                ClientInformation {
                    difficulty: 0,
                    ..information()
                }
            );
        }
    }

    #[test]
    fn test_chat_mode_out_of_byte_range_is_rejected_before_1_9() {
        let packet = SClientInformation {
            information: ClientInformation {
                chat_mode: 300,
                ..information()
            },
        };
        let mut buf = Vec::new();
        assert!(packet.encode(&mut buf, ProtocolVersion::V1_8).is_err());
        assert!(packet.encode(&mut buf, ProtocolVersion::V1_9).is_ok());
    }

    #[test]
    fn test_locale_longer_than_16_chars_is_rejected() {
        let mut bytes = vec![17];
        bytes.extend_from_slice(&[b'a'; 17]);
        bytes.extend_from_slice(&[0x0C, 0x00, 0x01, 0x7F, 0x01, 0x00, 0x01, 0x00]);
        assert!(
            SClientInformation::decode(&mut bytes.as_slice(), ProtocolVersion::V1_21_2).is_err()
        );
    }

    #[test]
    fn test_every_strict_prefix_fails_to_decode() {
        for version in [
            ProtocolVersion::V1_7_2,
            ProtocolVersion::V1_8,
            ProtocolVersion::V1_9,
            ProtocolVersion::V1_17,
            ProtocolVersion::V1_18,
            ProtocolVersion::V1_21_2,
        ] {
            let bytes = encoded(version);
            for len in 0..bytes.len() {
                assert!(
                    SClientInformation::decode(&mut &bytes[..len], version).is_err(),
                    "{version}: a {len}-byte prefix decoded"
                );
            }
        }
    }
}
