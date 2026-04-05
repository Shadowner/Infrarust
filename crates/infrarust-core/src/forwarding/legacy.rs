//! BungeeCord legacy forwarding.

use infrarust_protocol::packets::handshake::SHandshake;
use serde::Serialize;

use super::ForwardingData;

pub struct LegacyForwardingHandler;

impl LegacyForwardingHandler {
    pub fn apply_handshake(&self, handshake: &mut SHandshake, data: &ForwardingData) {
        let properties_json = serde_json::to_string(&properties_to_serializable(&data.properties))
            .unwrap_or_else(|_| "[]".to_string());

        handshake.server_address = format!(
            "{}\x00{}\x00{}\x00{}",
            handshake.server_address,
            data.real_ip,
            data.uuid.as_simple(),
            properties_json,
        );
    }
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
}
