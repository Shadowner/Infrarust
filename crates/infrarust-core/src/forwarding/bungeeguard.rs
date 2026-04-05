//! BungeeGuard forwarding.

use infrarust_api::types::ProfileProperty;
use infrarust_protocol::packets::handshake::SHandshake;

use super::ForwardingData;
use super::legacy::LegacyForwardingHandler;

pub struct BungeeGuardForwardingHandler {
    token: String,
}

impl BungeeGuardForwardingHandler {
    pub fn new(token: String) -> Self {
        Self { token }
    }

    pub fn apply_handshake(&self, handshake: &mut SHandshake, data: &ForwardingData) {
        let mut data_with_token = data.clone();
        data_with_token.properties.push(ProfileProperty {
            name: "bungeeguard-token".to_string(),
            value: self.token.clone(),
            signature: Some(String::new()),
        });
        LegacyForwardingHandler.apply_handshake(handshake, &data_with_token);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use infrarust_protocol::VarInt;
    use infrarust_protocol::version::ConnectionState;
    use std::net::IpAddr;

    #[test]
    fn test_bungeeguard_injects_token_as_property() {
        let data = ForwardingData {
            real_ip: "10.0.0.1".parse::<IpAddr>().unwrap(),
            uuid: uuid::Uuid::nil(),
            username: "Steve".to_string(),
            properties: vec![],
            protocol_version: infrarust_protocol::version::ProtocolVersion::V1_21,
            chat_session: None,
        };
        let mut handshake = SHandshake {
            protocol_version: VarInt(767),
            server_address: "mc.example.com".to_string(),
            server_port: 25565,
            next_state: ConnectionState::Login,
        };

        let handler = BungeeGuardForwardingHandler::new("my-secret-token".to_string());
        handler.apply_handshake(&mut handshake, &data);

        let parts: Vec<&str> = handshake.server_address.split('\x00').collect();
        assert_eq!(parts.len(), 4, "must be 4 segments (not 5)");
        assert_eq!(parts[0], "mc.example.com");
        assert_eq!(parts[1], "10.0.0.1");

        let props: Vec<serde_json::Value> = serde_json::from_str(parts[3]).unwrap();
        let token_prop = props
            .iter()
            .find(|p| p["name"] == "bungeeguard-token")
            .expect("bungeeguard-token property must exist");
        assert_eq!(token_prop["value"], "my-secret-token");
        assert_eq!(token_prop["signature"], "");
    }

    #[test]
    fn test_bungeeguard_preserves_existing_properties() {
        let data = ForwardingData {
            real_ip: "10.0.0.1".parse::<IpAddr>().unwrap(),
            uuid: uuid::Uuid::nil(),
            username: "Steve".to_string(),
            properties: vec![ProfileProperty {
                name: "textures".to_string(),
                value: "eyJ0ZXh0dXJlcw".to_string(),
                signature: Some("sig123".to_string()),
            }],
            protocol_version: infrarust_protocol::version::ProtocolVersion::V1_21,
            chat_session: None,
        };
        let mut handshake = SHandshake {
            protocol_version: VarInt(767),
            server_address: "mc.example.com".to_string(),
            server_port: 25565,
            next_state: ConnectionState::Login,
        };

        let handler = BungeeGuardForwardingHandler::new("secret123".to_string());
        handler.apply_handshake(&mut handshake, &data);

        let parts: Vec<&str> = handshake.server_address.split('\x00').collect();
        let props: Vec<serde_json::Value> = serde_json::from_str(parts[3]).unwrap();
        assert_eq!(props.len(), 2, "textures + bungeeguard-token");
        assert_eq!(props[0]["name"], "textures");
        assert_eq!(props[1]["name"], "bungeeguard-token");
        assert_eq!(props[1]["value"], "secret123");
    }
}
