use infrarust_api::types::Component;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

pub fn api_version(version: ProtocolVersion) -> infrarust_api::types::ProtocolVersion {
    infrarust_api::types::ProtocolVersion::new(version.0)
}

pub fn uses_nbt(version: ProtocolVersion, state: ConnectionState) -> bool {
    matches!(state, ConnectionState::Config | ConnectionState::Play)
        && version.no_less_than(ProtocolVersion::V1_20_3)
}

pub fn json_for(component: &Component, version: ProtocolVersion) -> String {
    component.to_json_for(api_version(version))
}

pub fn nbt_for(component: &Component, version: ProtocolVersion) -> Vec<u8> {
    component.to_nbt_for(api_version(version))
}

pub fn encode_text_component(
    component: &Component,
    version: ProtocolVersion,
    state: ConnectionState,
) -> Vec<u8> {
    if uses_nbt(version, state) {
        nbt_for(component, version)
    } else {
        json_for(component, version).into_bytes()
    }
}

pub fn decode_text_component(
    bytes: &[u8],
    version: ProtocolVersion,
    state: ConnectionState,
) -> Component {
    let parsed = if uses_nbt(version, state) {
        Component::from_nbt_network(bytes).ok()
    } else {
        std::str::from_utf8(bytes)
            .ok()
            .and_then(|json| Component::from_json(json).ok())
    };
    parsed.unwrap_or_else(|| Component::text(String::from_utf8_lossy(bytes)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use infrarust_api::types::{ClickEvent, HoverEvent, NamedColor};

    use super::*;

    const LOGIN: ConnectionState = ConnectionState::Login;
    const CONFIG: ConnectionState = ConnectionState::Config;
    const PLAY: ConnectionState = ConnectionState::Play;

    fn rich() -> Component {
        Component::text("Kicked: ")
            .color(NamedColor::Red)
            .append(Component::text("bye").bold())
            .click(ClickEvent::OpenUrl("https://example.com".into()))
            .hover(HoverEvent::show_text("why"))
    }

    #[test]
    fn login_reasons_are_json_in_every_version() {
        let json = br#"{"text":"Banned","color":"red"}"#;
        for version in [
            ProtocolVersion::V1_8,
            ProtocolVersion::V1_20_2,
            ProtocolVersion::V1_20_3,
            ProtocolVersion::V1_21_11,
        ] {
            let reason = decode_text_component(json, version, LOGIN);
            assert_eq!(
                reason,
                Component::text("Banned").color(NamedColor::Red),
                "{version:?}"
            );
        }
    }

    #[test]
    fn config_and_play_reasons_switch_to_nbt_at_1_20_3() {
        for state in [CONFIG, PLAY] {
            let json = br#"{"text":"Server closed"}"#;
            assert_eq!(
                decode_text_component(json, ProtocolVersion::V1_20_2, state),
                Component::text("Server closed")
            );
            let nbt =
                Component::text("Server closed").to_nbt_for(api_version(ProtocolVersion::V1_20_3));
            assert_eq!(
                decode_text_component(&nbt, ProtocolVersion::V1_20_3, state),
                Component::text("Server closed")
            );
        }
    }

    #[test]
    fn rich_reasons_round_trip_through_their_wire_form() {
        for version in [
            ProtocolVersion::V1_8,
            ProtocolVersion::V1_16,
            ProtocolVersion::V1_20_2,
            ProtocolVersion::V1_20_3,
            ProtocolVersion::V1_21_4,
            ProtocolVersion::V1_21_11,
        ] {
            for state in [LOGIN, CONFIG, PLAY] {
                let bytes = encode_text_component(&rich(), version, state);
                assert_eq!(
                    decode_text_component(&bytes, version, state),
                    rich(),
                    "{version:?} {state:?}"
                );
            }
        }
    }

    #[test]
    fn plain_strings_are_kept_as_text() {
        for (bytes, version, state) in [
            (&b"Disconnected"[..], ProtocolVersion::V1_8, PLAY),
            (&b"Disconnected"[..], ProtocolVersion::V1_21_11, LOGIN),
            (&b"backend disconnect"[..], ProtocolVersion::V1_21_11, PLAY),
            (&b""[..], ProtocolVersion::V1_21_11, CONFIG),
            (&b""[..], ProtocolVersion::V1_20_2, PLAY),
        ] {
            assert_eq!(
                decode_text_component(bytes, version, state),
                Component::text(String::from_utf8_lossy(bytes)),
                "{bytes:?}"
            );
        }
    }

    #[test]
    fn malformed_payloads_fall_back_to_their_raw_bytes() {
        let truncated = [0x0A, 0x08, 0x00, 0x04, b't', b'e'];
        assert_eq!(
            decode_text_component(&truncated, ProtocolVersion::V1_21_11, PLAY),
            Component::text(String::from_utf8_lossy(&truncated))
        );
        let invalid_utf8 = [b'{', 0xFF, 0xFE, b'}'];
        assert_eq!(
            decode_text_component(&invalid_utf8, ProtocolVersion::V1_20_2, PLAY),
            Component::text(String::from_utf8_lossy(&invalid_utf8))
        );
        let unclosed = br#"{"text":"half"#;
        assert_eq!(
            decode_text_component(unclosed, ProtocolVersion::V1_8, LOGIN),
            Component::text(r#"{"text":"half"#)
        );
        let mut trailing = Component::text("x").to_nbt_for(api_version(ProtocolVersion::V1_21_11));
        trailing.push(0x00);
        assert_eq!(
            decode_text_component(&trailing, ProtocolVersion::V1_21_11, PLAY),
            Component::text(String::from_utf8_lossy(&trailing))
        );
    }

    #[test]
    fn hostile_payloads_never_panic() {
        let deep_json = "[".repeat(100_000);
        let _ = decode_text_component(deep_json.as_bytes(), ProtocolVersion::V1_8, PLAY);
        let mut deep_nbt = Vec::new();
        for _ in 0..100_000 {
            deep_nbt.extend_from_slice(&[0x09, 0x09]);
            deep_nbt.extend_from_slice(&1i32.to_be_bytes());
        }
        let _ = decode_text_component(&deep_nbt, ProtocolVersion::V1_21_11, PLAY);
        let huge_list = [0x09, 0x08, 0x7F, 0xFF, 0xFF, 0xFF];
        let _ = decode_text_component(&huge_list, ProtocolVersion::V1_21_11, PLAY);
        for len in 0..64u8 {
            let noise: Vec<u8> = (0..len).map(|i| i.wrapping_mul(37) ^ len).collect();
            for state in [LOGIN, CONFIG, PLAY] {
                let _ = decode_text_component(&noise, ProtocolVersion::V1_21_11, state);
                let _ = decode_text_component(&noise, ProtocolVersion::V1_8, state);
            }
        }
    }

    #[test]
    fn encoding_follows_the_state_and_version() {
        let text = Component::text("hi");
        assert_eq!(
            encode_text_component(&text, ProtocolVersion::V1_21_11, LOGIN),
            json_for(&text, ProtocolVersion::V1_21_11).into_bytes()
        );
        assert_eq!(
            encode_text_component(&text, ProtocolVersion::V1_20_2, PLAY),
            br#"{"text":"hi"}"#.to_vec()
        );
        assert_eq!(
            encode_text_component(&text, ProtocolVersion::V1_20_3, CONFIG),
            text.to_nbt_for(api_version(ProtocolVersion::V1_20_3))
        );
        let clicked = Component::text("u").click(ClickEvent::OpenUrl("https://example.com".into()));
        let modern = String::from_utf8(encode_text_component(
            &clicked,
            ProtocolVersion::V1_21_11,
            LOGIN,
        ))
        .unwrap();
        assert!(modern.contains("click_event"), "{modern}");
        let older = json_for(&clicked, ProtocolVersion::V1_21_4);
        assert!(older.contains("clickEvent"), "{older}");
    }
}
