//! Velocity modern forwarding.

use hmac::{Hmac, Mac};
use infrarust_protocol::VarInt;
use infrarust_protocol::codec::McBufWriteExt;
use infrarust_protocol::packets::login::{CLoginPluginRequest, SLoginPluginResponse};
use infrarust_protocol::version::ProtocolVersion;
use sha2::Sha256;

use super::ForwardingData;

type HmacSha256 = Hmac<Sha256>;

const VELOCITY_PLAYER_INFO_CHANNEL: &str = "velocity:player_info";

/// Maximum forwarding version supported by Infrarust.
///
/// - `0x01` = MODERN_DEFAULT (1.13+)
/// - `0x02` = WITH_KEY (1.19)
/// - `0x03` = WITH_KEY_V2 (1.19.1)
const MAX_SUPPORTED_FORWARDING_VERSION: u8 = 0x04;

pub struct VelocityForwardingHandler;

impl VelocityForwardingHandler {
    fn negotiate_version(
        backend_version: u8,
        protocol: ProtocolVersion,
        has_chat_session: bool,
    ) -> u8 {
        let requested = backend_version.min(MAX_SUPPORTED_FORWARDING_VERSION);

        if requested <= 0x01 {
            return 0x01;
        }

        if protocol.no_less_than(ProtocolVersion::V1_19_3) {
            return if requested >= 0x04 { 0x04 } else { 0x01 };
        }

        if has_chat_session {
            return 0x02;
        }

        0x01
    }

    fn build_payload(data: &ForwardingData, version: u8) -> Vec<u8> {
        let mut buf = Vec::with_capacity(256);

        buf.write_var_int(&VarInt(i32::from(version)))
            .expect("writing to Vec<u8> cannot fail");
        buf.write_string(&data.real_ip.to_string())
            .expect("writing to Vec<u8> cannot fail");
        buf.write_uuid(&data.uuid)
            .expect("writing to Vec<u8> cannot fail");
        buf.write_string(&data.username)
            .expect("writing to Vec<u8> cannot fail");

        buf.write_var_int(&VarInt(data.properties.len() as i32))
            .expect("writing to Vec<u8> cannot fail");
        for prop in &data.properties {
            buf.write_string(&prop.name)
                .expect("writing to Vec<u8> cannot fail");
            buf.write_string(&prop.value)
                .expect("writing to Vec<u8> cannot fail");
            if let Some(ref sig) = prop.signature {
                buf.write_bool(true)
                    .expect("writing to Vec<u8> cannot fail");
                buf.write_string(sig)
                    .expect("writing to Vec<u8> cannot fail");
            } else {
                buf.write_bool(false)
                    .expect("writing to Vec<u8> cannot fail");
            }
        }

        if (0x02..0x04).contains(&version)
            && let Some(ref session) = data.chat_session
        {
            buf.extend_from_slice(&session.expiry.to_be_bytes());
            buf.write_byte_array(&session.public_key)
                .expect("writing to Vec<u8> cannot fail");
            buf.write_byte_array(&session.key_signature)
                .expect("writing to Vec<u8> cannot fail");

            if version >= 0x03 {
                if let Some(ref holder) = session.holder_uuid {
                    buf.write_bool(true)
                        .expect("writing to Vec<u8> cannot fail");
                    buf.write_uuid(holder)
                        .expect("writing to Vec<u8> cannot fail");
                } else {
                    buf.write_bool(false)
                        .expect("writing to Vec<u8> cannot fail");
                }
            }
        }

        buf
    }
}

fn sign_payload(secret: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size");
    mac.update(payload);
    let result = mac.finalize();
    let mut signature = [0u8; 32];
    signature.copy_from_slice(&result.into_bytes());
    signature
}

pub fn is_velocity_request(request: &CLoginPluginRequest) -> bool {
    request.channel == VELOCITY_PLAYER_INFO_CHANNEL
}

pub fn build_velocity_response(
    request: &CLoginPluginRequest,
    data: &ForwardingData,
    secret: &[u8],
) -> SLoginPluginResponse {
    let backend_version = request.data.first().copied().unwrap_or(0x01);
    let negotiated = VelocityForwardingHandler::negotiate_version(
        backend_version,
        data.protocol_version,
        data.chat_session.is_some(),
    );

    tracing::debug!(
        backend_version,
        negotiated,
        "velocity forwarding version negotiated"
    );

    let payload = VelocityForwardingHandler::build_payload(data, negotiated);
    let signature = sign_payload(secret, &payload);

    let mut response_data = Vec::with_capacity(32 + payload.len());
    response_data.extend_from_slice(&signature);
    response_data.extend(payload);

    SLoginPluginResponse {
        message_id: request.message_id,
        successful: true,
        data: response_data,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use hmac::Mac;
    use infrarust_api::types::ProfileProperty;
    use infrarust_protocol::version::ProtocolVersion;
    use std::net::IpAddr;

    fn make_data() -> ForwardingData {
        ForwardingData {
            real_ip: "192.168.1.42".parse::<IpAddr>().unwrap(),
            uuid: uuid::Uuid::parse_str("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap(),
            username: "Notch".to_string(),
            properties: vec![ProfileProperty {
                name: "textures".to_string(),
                value: "eyJ0aW1lc3RhbXAi".to_string(),
                signature: Some("Yp4ql3MT".to_string()),
            }],
            protocol_version: ProtocolVersion::V1_21,
            chat_session: None,
        }
    }

    #[test]
    fn test_velocity_version_negotiation_basic() {
        assert_eq!(
            VelocityForwardingHandler::negotiate_version(1, ProtocolVersion::V1_21, false),
            1
        );
        assert_eq!(
            VelocityForwardingHandler::negotiate_version(5, ProtocolVersion::V1_21, false),
            4
        );
        assert_eq!(
            VelocityForwardingHandler::negotiate_version(4, ProtocolVersion::V1_21, false),
            4
        );
    }

    #[test]
    fn test_velocity_version_negotiation_1_19_3_fallback() {
        assert_eq!(
            VelocityForwardingHandler::negotiate_version(3, ProtocolVersion::V1_19_3, false),
            1
        );
        assert_eq!(
            VelocityForwardingHandler::negotiate_version(2, ProtocolVersion::V1_19_3, false),
            1
        );
        assert_eq!(
            VelocityForwardingHandler::negotiate_version(4, ProtocolVersion::V1_19_3, false),
            4
        );
    }

    #[test]
    fn test_velocity_version_negotiation_pre_1_19_3_with_session() {
        assert_eq!(
            VelocityForwardingHandler::negotiate_version(3, ProtocolVersion::V1_19, true),
            2
        );
        assert_eq!(
            VelocityForwardingHandler::negotiate_version(2, ProtocolVersion::V1_19_1, true),
            2
        );
    }

    #[test]
    fn test_velocity_version_negotiation_pre_1_19_3_without_session() {
        assert_eq!(
            VelocityForwardingHandler::negotiate_version(3, ProtocolVersion::V1_19, false),
            1
        );
    }

    #[test]
    fn test_velocity_payload_v1_format() {
        let data = make_data();
        let payload = VelocityForwardingHandler::build_payload(&data, 0x01);

        assert_eq!(payload[0], 0x01);
        assert!(payload.len() > 20);
    }

    #[test]
    fn test_velocity_payload_v4_no_chat_session() {
        let data = make_data();
        let payload_v1 = VelocityForwardingHandler::build_payload(&data, 0x01);
        let payload_v4 = VelocityForwardingHandler::build_payload(&data, 0x04);

        assert_eq!(payload_v1[0], 0x01);
        assert_eq!(payload_v4[0], 0x04);
        assert_eq!(payload_v1[1..], payload_v4[1..]);
    }

    #[test]
    fn test_velocity_hmac_signature() {
        let secret = b"test-secret-key";
        let data = make_data();

        let payload = VelocityForwardingHandler::build_payload(&data, 0x01);
        let signature = sign_payload(secret, &payload);

        let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size");
        mac.update(&payload);
        assert!(mac.verify_slice(&signature).is_ok());

        let wrong_signature = sign_payload(b"wrong-key", &payload);
        assert_ne!(signature, wrong_signature);
    }

    #[test]
    fn test_velocity_signed_payload_structure() {
        let secret = b"my-secret";
        let data = make_data();

        let payload = VelocityForwardingHandler::build_payload(&data, 0x01);
        let signature = sign_payload(secret, &payload);

        let mut response = Vec::with_capacity(32 + payload.len());
        response.extend_from_slice(&signature);
        response.extend_from_slice(&payload);

        assert_eq!(response.len(), 32 + payload.len());
        assert_eq!(&response[..32], &signature);
        assert_eq!(&response[32..], &payload);

        let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size");
        mac.update(&response[32..]);
        assert!(mac.verify_slice(&response[..32]).is_ok());
    }

    #[test]
    fn test_velocity_payload_v2_with_chat_session() {
        use crate::forwarding::ChatSessionData;

        let mut data = make_data();
        data.protocol_version = ProtocolVersion::V1_19;
        data.chat_session = Some(ChatSessionData {
            expiry: 1_700_000_000_000i64,
            public_key: vec![0x30, 0x82, 0x01, 0x22],
            key_signature: vec![0xAA, 0xBB, 0xCC],
            holder_uuid: None,
        });

        let payload_v1 = VelocityForwardingHandler::build_payload(&data, 0x01);
        let payload_v2 = VelocityForwardingHandler::build_payload(&data, 0x02);

        assert!(
            payload_v2.len() > payload_v1.len(),
            "v2 payload ({}) must be larger than v1 ({})",
            payload_v2.len(),
            payload_v1.len()
        );
        assert_eq!(payload_v2[0], 0x02);
    }

    #[test]
    fn test_velocity_payload_v3_includes_holder_uuid() {
        use crate::forwarding::ChatSessionData;

        let mut data = make_data();
        data.protocol_version = ProtocolVersion::V1_19_1;
        data.chat_session = Some(ChatSessionData {
            expiry: 1_700_000_000_000i64,
            public_key: vec![0x30, 0x82],
            key_signature: vec![0xAA],
            holder_uuid: Some(
                uuid::Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap(),
            ),
        });

        let payload_v2 = VelocityForwardingHandler::build_payload(&data, 0x02);
        let payload_v3 = VelocityForwardingHandler::build_payload(&data, 0x03);

        assert!(
            payload_v3.len() > payload_v2.len(),
            "v3 payload ({}) must be larger than v2 ({})",
            payload_v3.len(),
            payload_v2.len()
        );
    }

    #[test]
    fn test_velocity_empty_login_plugin_request() {
        let secret = b"test-secret";
        let data = make_data();

        let request = CLoginPluginRequest {
            message_id: VarInt(42),
            channel: "velocity:player_info".to_string(),
            data: vec![],
        };

        let response = build_velocity_response(&request, &data, secret);
        assert_eq!(response.message_id, VarInt(42));
        assert!(response.successful);
        assert_eq!(response.data[32], 0x01);
    }

    #[test]
    fn test_velocity_hmac_wrong_secret_rejected() {
        let secret = b"correct-secret";
        let data = make_data();

        let payload = VelocityForwardingHandler::build_payload(&data, 0x01);
        let signature = sign_payload(secret, &payload);

        let mut mac =
            HmacSha256::new_from_slice(b"wrong-secret").expect("HMAC can take key of any size");
        mac.update(&payload);
        assert!(
            mac.verify_slice(&signature).is_err(),
            "HMAC with wrong secret must be rejected"
        );
    }
}
