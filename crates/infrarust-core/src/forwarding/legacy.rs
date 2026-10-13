//! BungeeCord legacy forwarding.

use infrarust_protocol::packets::handshake::SHandshake;
use serde::Serialize;

use super::ForwardingData;
use crate::util::{has_forge_marker, host_suffix};

const FORGE_CLIENT_PROPERTY: &str = "forgeClient";
const EXTRA_DATA_PROPERTY: &str = "extraData";

pub struct LegacyForwardingHandler;

impl LegacyForwardingHandler {
    pub fn apply_handshake(&self, handshake: &mut SHandshake, data: &ForwardingData) {
        let address = std::mem::take(&mut handshake.server_address);
        let suffix = host_suffix(&address);
        let host = &address[..address.len() - suffix.len()];

        let mut properties = properties_to_serializable(&data.properties);
        if has_forge_marker(suffix) {
            properties.extend(forge_properties(suffix));
        }
        let properties_json =
            serde_json::to_string(&properties).unwrap_or_else(|_| "[]".to_string());

        handshake.server_address = format!(
            "{}\x00{}\x00{}\x00{}",
            host,
            data.real_ip,
            data.uuid.as_simple(),
            properties_json,
        );
    }
}

fn forge_properties(suffix: &str) -> [SerializableProperty; 2] {
    [
        SerializableProperty {
            name: FORGE_CLIENT_PROPERTY.to_string(),
            value: "true".to_string(),
            signature: None,
        },
        SerializableProperty {
            name: EXTRA_DATA_PROPERTY.to_string(),
            value: suffix.replace('\0', "\u{1}"),
            signature: Some(String::new()),
        },
    ]
}

#[derive(Serialize)]
#[cfg_attr(test, derive(serde::Deserialize))]
struct SerializableProperty {
    name: String,
    value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
}

fn properties_to_serializable(
    props: &[infrarust_api::types::ProfileProperty],
) -> Vec<SerializableProperty> {
    props
        .iter()
        .map(|p| SerializableProperty {
            name: p.name.clone(),
            value: p.value.clone(),
            signature: p.signature.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use infrarust_api::types::ProfileProperty;
    use infrarust_protocol::VarInt;
    use infrarust_protocol::version::ConnectionState;
    use serde::Deserialize;
    use std::net::IpAddr;

    fn make_data(properties: Vec<ProfileProperty>) -> ForwardingData {
        ForwardingData {
            real_ip: "192.168.1.42".parse::<IpAddr>().unwrap(),
            uuid: uuid::Uuid::parse_str("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap(),
            username: "Notch".to_string(),
            properties,
            protocol_version: infrarust_protocol::version::ProtocolVersion::V1_21,
            chat_session: None,
        }
    }

    fn make_handshake() -> SHandshake {
        SHandshake {
            protocol_version: VarInt(767),
            server_address: "play.example.com".to_string(),
            server_port: 25565,
            next_state: ConnectionState::Login,
            transfer: false,
        }
    }

    #[test]
    fn test_legacy_forwarding_injects_ip_uuid_properties() {
        let data = make_data(vec![ProfileProperty {
            name: "textures".to_string(),
            value: "base64data".to_string(),
            signature: Some("sig123".to_string()),
        }]);
        let mut handshake = make_handshake();

        LegacyForwardingHandler.apply_handshake(&mut handshake, &data);

        let parts: Vec<&str> = handshake.server_address.split('\x00').collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "play.example.com");
        assert_eq!(parts[1], "192.168.1.42");
        assert_eq!(parts[2], "069a79f444e94726a5befca90e38aaf5");
    }

    #[test]
    fn test_legacy_forwarding_uuid_format_no_dashes() {
        let data = make_data(vec![]);
        let mut handshake = make_handshake();

        LegacyForwardingHandler.apply_handshake(&mut handshake, &data);

        let parts: Vec<&str> = handshake.server_address.split('\x00').collect();
        let uuid_part = parts[2];
        assert!(!uuid_part.contains('-'));
        assert_eq!(uuid_part.len(), 32);
    }

    #[test]
    fn test_legacy_forwarding_empty_properties() {
        let data = make_data(vec![]);
        let mut handshake = make_handshake();

        LegacyForwardingHandler.apply_handshake(&mut handshake, &data);

        let parts: Vec<&str> = handshake.server_address.split('\x00').collect();
        let json: serde_json::Value = serde_json::from_str(parts[3]).unwrap();
        assert_eq!(json, serde_json::json!([]));
    }

    #[derive(Deserialize)]
    struct TestProperty {
        name: String,
        value: String,
        signature: Option<String>,
    }

    #[test]
    fn test_legacy_forwarding_with_textures() {
        let data = make_data(vec![ProfileProperty {
            name: "textures".to_string(),
            value: "eyJ0aW1lc3RhbXAi".to_string(),
            signature: Some("Yp4ql3MT".to_string()),
        }]);
        let mut handshake = make_handshake();

        LegacyForwardingHandler.apply_handshake(&mut handshake, &data);

        let parts: Vec<&str> = handshake.server_address.split('\x00').collect();
        let json: Vec<TestProperty> = serde_json::from_str(parts[3]).unwrap();
        assert_eq!(json.len(), 1);
        assert_eq!(json[0].name, "textures");
        assert_eq!(json[0].value, "eyJ0aW1lc3RhbXAi");
        assert_eq!(json[0].signature.as_deref(), Some("Yp4ql3MT"));
    }

    fn forwarded(server_address: &str, properties: Vec<ProfileProperty>) -> SHandshake {
        let mut handshake = make_handshake();
        handshake.server_address = server_address.to_string();
        LegacyForwardingHandler.apply_handshake(&mut handshake, &make_data(properties));
        handshake
    }

    fn forwarded_properties(handshake: &SHandshake) -> Vec<TestProperty> {
        let parts: Vec<&str> = handshake.server_address.split('\x00').collect();
        assert_eq!(parts.len(), 4, "{parts:?}");
        assert_eq!(parts[1], "192.168.1.42");
        assert_eq!(parts[2], "069a79f444e94726a5befca90e38aaf5");
        serde_json::from_str(parts[3]).unwrap()
    }

    fn assert_forge_properties(properties: &[TestProperty], marker: &str) {
        let [forge_client, extra_data] = properties else {
            panic!(
                "expected the two forge properties, got {} properties",
                properties.len()
            );
        };
        assert_eq!(forge_client.name, "forgeClient");
        assert_eq!(forge_client.value, "true");
        assert_eq!(forge_client.signature, None);
        assert_eq!(extra_data.name, "extraData");
        assert_eq!(extra_data.value, marker.replace('\0', "\u{1}"));
        assert_eq!(extra_data.signature.as_deref(), Some(""));
    }

    #[test]
    fn a_forge_marker_becomes_profile_properties() {
        for marker in ["\0FML\0", "\0FML2\0", "\0FML3\0", "\0FORGE"] {
            let handshake = forwarded(&format!("play.example.com{marker}"), vec![]);
            assert!(
                handshake
                    .server_address
                    .starts_with("play.example.com\x00192.168.1.42\0")
            );
            assert_forge_properties(&forwarded_properties(&handshake), marker);
        }
    }

    #[test]
    fn forge_properties_follow_the_profile_properties() {
        let handshake = forwarded(
            "play.example.com\0FML3\0",
            vec![ProfileProperty {
                name: "textures".to_string(),
                value: "skin".to_string(),
                signature: Some("sig".to_string()),
            }],
        );
        let properties = forwarded_properties(&handshake);
        assert_eq!(properties.len(), 3);
        assert_eq!(properties[0].name, "textures");
        assert_forge_properties(&properties[1..], "\0FML3\0");
    }

    #[test]
    fn a_suffix_without_a_forge_marker_is_dropped() {
        let handshake = forwarded("play.example.com\0not-a-marker\0", vec![]);
        assert!(
            handshake
                .server_address
                .starts_with("play.example.com\x00192.168.1.42\0")
        );
        assert!(forwarded_properties(&handshake).is_empty());
    }

    #[test]
    fn a_forged_suffix_cannot_inject_forwarding_fields() {
        let injected = "\0FML2\0\x00198.51.100.7\x000123456789abcdef0123456789abcdef";
        let handshake = forwarded(&format!("play.example.com{injected}"), vec![]);
        assert_eq!(handshake.server_address.matches('\0').count(), 3);
        assert!(
            handshake
                .server_address
                .starts_with("play.example.com\x00192.168.1.42\0")
        );
        assert_forge_properties(&forwarded_properties(&handshake), injected);
    }

    #[test]
    fn a_forged_suffix_without_a_marker_injects_nothing() {
        let injected = "\x00198.51.100.7\x000123456789abcdef0123456789abcdef\0[{\"name\":\"x\",\"value\":\"y\"}]";
        let handshake = forwarded(&format!("play.example.com{injected}"), vec![]);
        assert_eq!(handshake.server_address.matches('\0').count(), 3);
        assert!(!handshake.server_address.contains("198.51.100.7"));
        assert!(forwarded_properties(&handshake).is_empty());
    }
}
