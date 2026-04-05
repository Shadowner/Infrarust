use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::{ProtocolError, ProtocolResult};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

use super::Packet;

/// A game profile property (textures, skin data, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

fn read_byte_array_short(r: &mut &[u8]) -> ProtocolResult<Vec<u8>> {
    let len = r.read_i16_be()?;
    if len < 0 {
        return Err(ProtocolError::invalid("negative byte array length"));
    }
    r.read_byte_array_bounded(len as usize)
}

fn write_byte_array_short(
    mut w: &mut (impl std::io::Write + ?Sized),
    data: &[u8],
) -> ProtocolResult<()> {
    if data.len() > i16::MAX as usize {
        return Err(ProtocolError::too_large(i16::MAX as usize, data.len()));
    }
    w.write_i16_be(data.len() as i16)?;
    w.write_all(data)?;
    Ok(())
}

#[allow(clippy::many_single_char_names)] // UUID components are conventionally named a/b/c/d
fn read_uuid_int_array(r: &mut &[u8]) -> ProtocolResult<uuid::Uuid> {
    let a = r.read_i32_be()? as u32;
    let b = r.read_i32_be()? as u32;
    let c = r.read_i32_be()? as u32;
    let d = r.read_i32_be()? as u32;
    let msb = (u64::from(a) << 32) | u64::from(b);
    let lsb = (u64::from(c) << 32) | u64::from(d);
    Ok(uuid::Uuid::from_u128(
        (u128::from(msb) << 64) | u128::from(lsb),
    ))
}

fn write_uuid_int_array(
    mut w: &mut (impl std::io::Write + ?Sized),
    uuid: &uuid::Uuid,
) -> ProtocolResult<()> {
    let val = uuid.as_u128();
    let msb = (val >> 64) as u64;
    let lsb = val as u64;
    w.write_i32_be((msb >> 32) as i32)?;
    w.write_i32_be(msb as i32)?;
    w.write_i32_be((lsb >> 32) as i32)?;
    w.write_i32_be(lsb as i32)?;
    Ok(())
}

/// Login start packet (Serverbound, 0x00).
///
/// First packet sent by the client after a Handshake with login intent.
/// The format varies significantly across versions:
/// - < 1.19: name only
/// - 1.19 - 1.19.2: name + optional player key + optional uuid
/// - 1.19.3 - 1.20.1: name + optional uuid
/// - 1.20.2+: name + mandatory uuid
///
/// The player key (1.19-1.19.2) is stored as opaque bytes since the proxy
/// forwards it without interpretation.
#[derive(Debug, Clone)]
pub struct SLoginStart {
    pub name: String,
    pub uuid: Option<uuid::Uuid>,
    /// Opaque player key blob from 1.19-1.19.2 (bool prefix already consumed).
    pub signature_data: Option<Vec<u8>>,
}

impl Packet for SLoginStart {
    const NAME: &'static str = "SLoginStart";

    fn state() -> ConnectionState {
        ConnectionState::Login
    }

    fn direction() -> Direction {
        Direction::Serverbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let name = r.read_string_bounded(16)?;
        if name.is_empty() {
            return Err(ProtocolError::invalid("empty username in SLoginStart"));
        }

        let mut uuid = None;
        let mut signature_data = None;

        if version.no_less_than(ProtocolVersion::V1_19) {
            // Player key: exists in 1.19 - 1.19.2 only (removed in 1.19.3)
            if version.less_than(ProtocolVersion::V1_19_3) && r.read_bool()? {
                // Read the player key as opaque bytes
                let mut key_buf = Vec::new();
                // timestamp (i64)
                let ts = r.read_i64_be()?;
                key_buf.extend_from_slice(&ts.to_be_bytes());
                // public key (byte array)
                let pk = r.read_byte_array(512)?;
                VarInt(pk.len() as i32).encode(&mut key_buf)?;
                key_buf.extend_from_slice(&pk);
                // signature (byte array)
                let sig = r.read_byte_array(4096)?;
                VarInt(sig.len() as i32).encode(&mut key_buf)?;
                key_buf.extend_from_slice(&sig);
                signature_data = Some(key_buf);
            }

            // UUID handling: mandatory in 1.20.2+, optional (bool-prefixed) in 1.19.1+
            let has_uuid = if version.no_less_than(ProtocolVersion::V1_20_2) {
                true
            } else if version.no_less_than(ProtocolVersion::V1_19_1) {
                r.read_bool()?
            } else {
                false
            };
            if has_uuid {
                uuid = Some(r.read_uuid()?);
            }
        }

        Ok(Self {
            name,
            uuid,
            signature_data,
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_string(&self.name)?;

        if version.no_less_than(ProtocolVersion::V1_19) {
            // Player key: 1.19 - 1.19.2 only
            if version.less_than(ProtocolVersion::V1_19_3) {
                if let Some(ref sig) = self.signature_data {
                    w.write_bool(true)?;
                    w.write_all(sig)?;
                } else {
                    w.write_bool(false)?;
                }
            }

            // UUID
            if version.no_less_than(ProtocolVersion::V1_20_2) {
                let uuid = self.uuid.unwrap_or(uuid::Uuid::nil());
                w.write_uuid(&uuid)?;
            } else if version.no_less_than(ProtocolVersion::V1_19_1) {
                if let Some(ref uuid) = self.uuid {
                    w.write_bool(true)?;
                    w.write_uuid(uuid)?;
                } else {
                    w.write_bool(false)?;
                }
            }
        }

        Ok(())
    }
}

/// Encryption request packet (Clientbound, 0x01).
///
/// Sent by the server to initiate encryption. Contains the server's RSA
/// public key and a verify token.
#[derive(Debug, Clone)]
pub struct CEncryptionRequest {
    pub server_id: String,
    pub public_key: Vec<u8>,
    pub verify_token: Vec<u8>,
    /// Whether the server requires Mojang authentication. Only sent in 1.20.5+.
    pub should_authenticate: bool,
}

impl Packet for CEncryptionRequest {
    const NAME: &'static str = "CEncryptionRequest";

    fn state() -> ConnectionState {
        ConnectionState::Login
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let server_id = r.read_string_bounded(20)?;

        let (public_key, verify_token) = if version.no_less_than(ProtocolVersion::V1_8) {
            let pk = r.read_byte_array(256)?;
            let vt = r.read_byte_array(16)?;
            (pk, vt)
        } else {
            let pk = read_byte_array_short(r)?;
            let vt = read_byte_array_short(r)?;
            (pk, vt)
        };

        let should_authenticate = if version.no_less_than(ProtocolVersion::V1_20_5) {
            r.read_bool()?
        } else {
            true
        };

        Ok(Self {
            server_id,
            public_key,
            verify_token,
            should_authenticate,
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_string(&self.server_id)?;

        if version.no_less_than(ProtocolVersion::V1_8) {
            w.write_byte_array(&self.public_key)?;
            w.write_byte_array(&self.verify_token)?;
        } else {
            write_byte_array_short(w, &self.public_key)?;
            write_byte_array_short(w, &self.verify_token)?;
        }

        if version.no_less_than(ProtocolVersion::V1_20_5) {
            w.write_bool(self.should_authenticate)?;
        }

        Ok(())
    }
}

/// Encryption response packet (Serverbound, 0x01).
///
/// Client's response with the encrypted shared secret and verify token.
/// In 1.19-1.19.2, an optional salt field exists for message signing.
#[derive(Debug, Clone)]
pub struct SEncryptionResponse {
    pub shared_secret: Vec<u8>,
    pub verify_token: Vec<u8>,
    /// Salt for message signing, only present in 1.19 - 1.19.2 when
    /// the client chose salt-based verification instead of token-based.
    pub salt: Option<i64>,
}

impl Packet for SEncryptionResponse {
    const NAME: &'static str = "SEncryptionResponse";

    fn state() -> ConnectionState {
        ConnectionState::Login
    }

    fn direction() -> Direction {
        Direction::Serverbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let mut salt = None;

        if version.no_less_than(ProtocolVersion::V1_8) {
            let shared_secret = r.read_byte_array(128)?;

            // 1.19 - 1.19.2: optional salt before verify token
            if version.no_less_than(ProtocolVersion::V1_19)
                && version.less_than(ProtocolVersion::V1_19_3)
                && !r.read_bool()?
            {
                salt = Some(r.read_i64_be()?);
            }

            let max_vt = if version.no_less_than(ProtocolVersion::V1_19) {
                256
            } else {
                128
            };
            let verify_token = r.read_byte_array(max_vt)?;

            Ok(Self {
                shared_secret,
                verify_token,
                salt,
            })
        } else {
            let shared_secret = read_byte_array_short(r)?;
            let verify_token = read_byte_array_short(r)?;
            Ok(Self {
                shared_secret,
                verify_token,
                salt: None,
            })
        }
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        if version.no_less_than(ProtocolVersion::V1_8) {
            w.write_byte_array(&self.shared_secret)?;

            if version.no_less_than(ProtocolVersion::V1_19)
                && version.less_than(ProtocolVersion::V1_19_3)
            {
                if let Some(salt) = self.salt {
                    w.write_bool(false)?;
                    w.write_i64_be(salt)?;
                } else {
                    w.write_bool(true)?;
                }
            }

            w.write_byte_array(&self.verify_token)?;
        } else {
            write_byte_array_short(w, &self.shared_secret)?;
            write_byte_array_short(w, &self.verify_token)?;
        }

        Ok(())
    }
}

/// Set compression packet (Clientbound, 0x03).
///
/// Informs the client of the compression threshold. After this packet,
/// all subsequent packets use the compressed format.
#[derive(Debug, Clone)]
pub struct CSetCompression {
    pub threshold: VarInt,
}

impl Packet for CSetCompression {
    const NAME: &'static str = "CSetCompression";

    fn state() -> ConnectionState {
        ConnectionState::Login
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let threshold = r.read_var_int()?;
        Ok(Self { threshold })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_var_int(&self.threshold)?;
        Ok(())
    }
}

/// Login success packet (Clientbound, 0x02).
///
/// The UUID format varies significantly across versions:
/// - < 1.7.6: UUID as undashed string
/// - 1.7.6 - 1.15: UUID as dashed string
/// - 1.16 - 1.18.2: UUID as int array (4x i32)
/// - 1.19+: UUID as binary (16 bytes)
///
/// Properties (skin/texture data) are only sent in 1.19+.
/// `strict_error_handling` is only sent in 1.20.5 and 1.21.
#[derive(Debug, Clone)]
pub struct CLoginSuccess {
    pub uuid: uuid::Uuid,
    pub username: String,
    pub properties: Vec<Property>,
    pub strict_error_handling: bool,
}

impl Packet for CLoginSuccess {
    const NAME: &'static str = "CLoginSuccess";

    fn state() -> ConnectionState {
        ConnectionState::Login
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        // Decode UUID based on version
        let uuid = if version.no_less_than(ProtocolVersion::V1_19) {
            r.read_uuid()?
        } else if version.no_less_than(ProtocolVersion::V1_16) {
            read_uuid_int_array(r)?
        } else if version.no_less_than(ProtocolVersion::V1_7_6) {
            let s = r.read_string_bounded(36)?;
            uuid::Uuid::parse_str(&s)
                .map_err(|e| ProtocolError::invalid(format!("invalid UUID string: {e}")))?
        } else {
            let s = r.read_string_bounded(32)?;
            uuid::Uuid::parse_str(&s)
                .map_err(|e| ProtocolError::invalid(format!("invalid undashed UUID: {e}")))?
        };

        let username = r.read_string_bounded(16)?;

        // Properties: 1.19+
        let properties = if version.no_less_than(ProtocolVersion::V1_19) {
            let count = r.read_var_int()?.0 as usize;
            let mut props = Vec::with_capacity(count.min(64));
            for _ in 0..count {
                let name = r.read_string()?;
                let value = r.read_string()?;
                let signature = if r.read_bool()? {
                    Some(r.read_string()?)
                } else {
                    None
                };
                props.push(Property {
                    name,
                    value,
                    signature,
                });
            }
            props
        } else {
            Vec::new()
        };

        // strict_error_handling: only 1.20.5 and 1.21
        let strict_error_handling =
            if version == ProtocolVersion::V1_20_5 || version == ProtocolVersion::V1_21 {
                r.read_bool()?
            } else {
                false
            };

        Ok(Self {
            uuid,
            username,
            properties,
            strict_error_handling,
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        // Encode UUID based on version
        if version.no_less_than(ProtocolVersion::V1_19) {
            w.write_uuid(&self.uuid)?;
        } else if version.no_less_than(ProtocolVersion::V1_16) {
            write_uuid_int_array(w, &self.uuid)?;
        } else if version.no_less_than(ProtocolVersion::V1_7_6) {
            w.write_string(&self.uuid.to_string())?;
        } else {
            w.write_string(&self.uuid.as_simple().to_string())?;
        }

        w.write_string(&self.username)?;

        // Properties: 1.19+
        if version.no_less_than(ProtocolVersion::V1_19) {
            // Property count bounded by protocol
            w.write_var_int(&VarInt(self.properties.len() as i32))?;
            for prop in &self.properties {
                w.write_string(&prop.name)?;
                w.write_string(&prop.value)?;
                if let Some(ref sig) = prop.signature {
                    w.write_bool(true)?;
                    w.write_string(sig)?;
                } else {
                    w.write_bool(false)?;
                }
            }
        }

        // strict_error_handling: only 1.20.5 and 1.21
        if version == ProtocolVersion::V1_20_5 || version == ProtocolVersion::V1_21 {
            w.write_bool(self.strict_error_handling)?;
        }

        Ok(())
    }
}

/// Login disconnect packet (Clientbound, 0x00).
///
/// Sent by the server to disconnect the client during login with a reason.
#[derive(Debug, Clone)]
pub struct CLoginDisconnect {
    pub reason: String,
}

impl Packet for CLoginDisconnect {
    const NAME: &'static str = "CLoginDisconnect";

    fn state() -> ConnectionState {
        ConnectionState::Login
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let reason = r.read_string()?;
        Ok(Self { reason })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_string(&self.reason)?;
        Ok(())
    }
}

/// Login plugin request packet (Clientbound, 0x04, 1.13+).
///
/// Used for custom login channels (e.g., Velocity modern forwarding).
#[derive(Debug, Clone)]
pub struct CLoginPluginRequest {
    pub message_id: VarInt,
    pub channel: String,
    pub data: Vec<u8>,
}

impl Packet for CLoginPluginRequest {
    const NAME: &'static str = "CLoginPluginRequest";

    fn state() -> ConnectionState {
        ConnectionState::Login
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let message_id = r.read_var_int()?;
        let channel = r.read_string()?;
        let data = r.read_remaining()?;
        Ok(Self {
            message_id,
            channel,
            data,
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_var_int(&self.message_id)?;
        w.write_string(&self.channel)?;
        w.write_all(&self.data)?;
        Ok(())
    }
}

/// Login plugin response packet (Serverbound, 0x02, 1.13+).
///
/// Client's response to a login plugin request.
#[derive(Debug, Clone)]
pub struct SLoginPluginResponse {
    pub message_id: VarInt,
    pub successful: bool,
    pub data: Vec<u8>,
}

impl Packet for SLoginPluginResponse {
    const NAME: &'static str = "SLoginPluginResponse";

    fn state() -> ConnectionState {
        ConnectionState::Login
    }

    fn direction() -> Direction {
        Direction::Serverbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let message_id = r.read_var_int()?;
        let successful = r.read_bool()?;
        let data = r.read_remaining()?;
        Ok(Self {
            message_id,
            successful,
            data,
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_var_int(&self.message_id)?;
        w.write_bool(self.successful)?;
        w.write_all(&self.data)?;
        Ok(())
    }
}

/// Login acknowledged packet (Serverbound, 0x03, 1.20.2+).
///
/// Empty packet marking the transition from Login to Config state.
#[derive(Debug, Clone)]
pub struct SLoginAcknowledged;

impl Packet for SLoginAcknowledged {
    const NAME: &'static str = "SLoginAcknowledged";

    fn state() -> ConnectionState {
        ConnectionState::Login
    }

    fn direction() -> Direction {
        Direction::Serverbound
    }

    fn decode(_r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        Ok(Self)
    }

    fn encode(
        &self,
        _w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::registry::build_default_registry;

    fn round_trip<P: Packet>(packet: &P, version: ProtocolVersion) -> P {
        let mut buf = Vec::new();
        packet.encode(&mut buf, version).unwrap();
        P::decode(&mut buf.as_slice(), version).unwrap()
    }

    #[test]
    fn test_login_start_v1_8_name_only() {
        let pkt = SLoginStart {
            name: "Notch".to_string(),
            uuid: None,
            signature_data: None,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_8);
        assert_eq!(decoded.name, "Notch");
        assert!(decoded.uuid.is_none());
        assert!(decoded.signature_data.is_none());
    }

    #[test]
    fn test_login_start_v1_20_2_name_and_uuid() {
        let uuid = uuid::Uuid::parse_str("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap();
        let pkt = SLoginStart {
            name: "Notch".to_string(),
            uuid: Some(uuid),
            signature_data: None,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_20_2);
        assert_eq!(decoded.name, "Notch");
        assert_eq!(decoded.uuid, Some(uuid));
    }

    #[test]
    fn test_login_success_v1_8_uuid_string() {
        let uuid = uuid::Uuid::parse_str("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap();
        let pkt = CLoginSuccess {
            uuid,
            username: "Notch".to_string(),
            properties: Vec::new(),
            strict_error_handling: false,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_8);
        assert_eq!(decoded.uuid, uuid);
        assert_eq!(decoded.username, "Notch");
        assert!(decoded.properties.is_empty());
    }

    #[test]
    fn test_login_success_v1_19_binary_uuid_with_properties() {
        let uuid = uuid::Uuid::parse_str("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap();
        let pkt = CLoginSuccess {
            uuid,
            username: "Notch".to_string(),
            properties: vec![
                Property {
                    name: "textures".to_string(),
                    value: "base64data".to_string(),
                    signature: Some("sig123".to_string()),
                },
                Property {
                    name: "other".to_string(),
                    value: "val".to_string(),
                    signature: None,
                },
            ],
            strict_error_handling: false,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_19);
        assert_eq!(decoded.uuid, uuid);
        assert_eq!(decoded.username, "Notch");
        assert_eq!(decoded.properties.len(), 2);
        assert_eq!(decoded.properties[0].name, "textures");
        assert_eq!(decoded.properties[0].signature, Some("sig123".to_string()));
        assert!(decoded.properties[1].signature.is_none());
    }

    #[test]
    fn test_login_success_v1_20_5_strict_error_handling() {
        let uuid = uuid::Uuid::parse_str("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap();
        let pkt = CLoginSuccess {
            uuid,
            username: "Notch".to_string(),
            properties: Vec::new(),
            strict_error_handling: true,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_20_5);
        assert!(decoded.strict_error_handling);

        // V1_21_2 should NOT have strict_error_handling
        let decoded_new = round_trip(&pkt, ProtocolVersion::V1_21_2);
        assert!(!decoded_new.strict_error_handling);
    }

    #[test]
    fn test_login_success_v1_16_uuid_int_array() {
        let uuid = uuid::Uuid::parse_str("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap();
        let pkt = CLoginSuccess {
            uuid,
            username: "Notch".to_string(),
            properties: Vec::new(),
            strict_error_handling: false,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_16);
        assert_eq!(decoded.uuid, uuid);
        assert_eq!(decoded.username, "Notch");
    }

    #[test]
    fn test_encryption_request_v1_8() {
        let pkt = CEncryptionRequest {
            server_id: String::new(),
            public_key: vec![1, 2, 3, 4, 5],
            verify_token: vec![10, 20, 30, 40],
            should_authenticate: true,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_8);
        assert!(decoded.server_id.is_empty());
        assert_eq!(decoded.public_key, vec![1, 2, 3, 4, 5]);
        assert_eq!(decoded.verify_token, vec![10, 20, 30, 40]);
        assert!(decoded.should_authenticate); // default true for < 1.20.5
    }

    #[test]
    fn test_encryption_request_v1_20_5_with_should_authenticate() {
        let pkt = CEncryptionRequest {
            server_id: String::new(),
            public_key: vec![1, 2, 3],
            verify_token: vec![4, 5, 6],
            should_authenticate: false,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_20_5);
        assert!(!decoded.should_authenticate);
    }

    #[test]
    fn test_encryption_response_v1_8() {
        let pkt = SEncryptionResponse {
            shared_secret: vec![0xAB; 16],
            verify_token: vec![0xCD; 4],
            salt: None,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_8);
        assert_eq!(decoded.shared_secret, vec![0xAB; 16]);
        assert_eq!(decoded.verify_token, vec![0xCD; 4]);
        assert!(decoded.salt.is_none());
    }

    #[test]
    fn test_set_compression_round_trip() {
        let pkt = CSetCompression {
            threshold: VarInt(256),
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_8);
        assert_eq!(decoded.threshold, VarInt(256));

        // Negative threshold = disable compression
        let pkt2 = CSetCompression {
            threshold: VarInt(-1),
        };
        let decoded2 = round_trip(&pkt2, ProtocolVersion::V1_21);
        assert_eq!(decoded2.threshold, VarInt(-1));
    }

    #[test]
    fn test_login_disconnect_round_trip() {
        let reason = r#"{"text":"You are banned!"}"#;
        let pkt = CLoginDisconnect {
            reason: reason.to_string(),
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.reason, reason);
    }

    #[test]
    fn test_login_plugin_request_response_round_trip() {
        let req = CLoginPluginRequest {
            message_id: VarInt(42),
            channel: "velocity:player_info".to_string(),
            data: vec![1, 2, 3, 4, 5],
        };
        let decoded_req = round_trip(&req, ProtocolVersion::V1_13);
        assert_eq!(decoded_req.message_id, VarInt(42));
        assert_eq!(decoded_req.channel, "velocity:player_info");
        assert_eq!(decoded_req.data, vec![1, 2, 3, 4, 5]);

        let resp = SLoginPluginResponse {
            message_id: VarInt(42),
            successful: true,
            data: vec![6, 7, 8],
        };
        let decoded_resp = round_trip(&resp, ProtocolVersion::V1_13);
        assert_eq!(decoded_resp.message_id, VarInt(42));
        assert!(decoded_resp.successful);
        assert_eq!(decoded_resp.data, vec![6, 7, 8]);
    }

    #[test]
    fn test_login_acknowledged_round_trip() {
        let pkt = SLoginAcknowledged;
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_20_2).unwrap();
        assert!(buf.is_empty());
        SLoginAcknowledged::decode(&mut buf.as_slice(), ProtocolVersion::V1_20_2).unwrap();
    }

    #[test]
    fn test_login_acknowledged_only_registered_for_1_20_2_plus() {
        let registry = build_default_registry();

        // Should NOT be registered for V1_20
        assert!(
            registry
                .get_packet_id::<SLoginAcknowledged>(
                    ConnectionState::Login,
                    Direction::Serverbound,
                    ProtocolVersion::V1_20,
                )
                .is_none(),
            "SLoginAcknowledged should not be registered for V1_20"
        );

        // Should be registered for V1_20_2+
        assert!(
            registry
                .get_packet_id::<SLoginAcknowledged>(
                    ConnectionState::Login,
                    Direction::Serverbound,
                    ProtocolVersion::V1_20_2,
                )
                .is_some(),
            "SLoginAcknowledged should be registered for V1_20_2"
        );
    }
}
