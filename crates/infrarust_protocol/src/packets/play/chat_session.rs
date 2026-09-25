use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::login::ProfileKey;
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

const PUBLIC_KEY_MAX_LEN: usize = 512;

const KEY_SIGNATURE_MAX_LEN: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SChatSessionUpdate {
    pub session_id: uuid::Uuid,
    pub profile_key: ProfileKey,
}

impl Packet for SChatSessionUpdate {
    const NAME: &'static str = "SChatSessionUpdate";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Serverbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_19_3 => 0x20,
        V1_19_4 => 0x06,
        V1_20_5 => 0x07,
        V1_21_2 => 0x08,
        V1_21_6 => 0x09,
        V26_1   => 0x0A,
    ];

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let session_id = r.read_uuid()?;
        let expires_at = r.read_i64_be()?;
        let public_key = r.read_byte_array(PUBLIC_KEY_MAX_LEN)?;
        let key_signature = r.read_byte_array(KEY_SIGNATURE_MAX_LEN)?;
        Ok(Self {
            session_id,
            profile_key: ProfileKey {
                expires_at,
                public_key,
                key_signature,
            },
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_uuid(&self.session_id)?;
        w.write_i64_be(self.profile_key.expires_at)?;
        w.write_byte_array(&self.profile_key.public_key)?;
        w.write_byte_array(&self.profile_key.key_signature)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn session() -> SChatSessionUpdate {
        SChatSessionUpdate {
            session_id: uuid::Uuid::from_u128(0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10),
            profile_key: ProfileKey {
                expires_at: 0x0000_0190_0000_0001,
                public_key: vec![0x30, 0x82],
                key_signature: vec![0xEE; 3],
            },
        }
    }

    const SESSION_BYTES: &[u8] = &[
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10, 0x00, 0x00, 0x01, 0x90, 0x00, 0x00, 0x00, 0x01, 0x02, 0x30, 0x82, 0x03, 0xEE, 0xEE,
        0xEE,
    ];

    #[test]
    fn test_session_update_golden_bytes() {
        for version in [
            ProtocolVersion::V1_19_3,
            ProtocolVersion::V1_20_5,
            ProtocolVersion::V1_21_11,
        ] {
            let mut buf = Vec::new();
            session().encode(&mut buf, version).unwrap();
            assert_eq!(buf, SESSION_BYTES, "{version}");

            let mut r = SESSION_BYTES;
            assert_eq!(
                SChatSessionUpdate::decode(&mut r, version).unwrap(),
                session()
            );
            assert!(r.is_empty());
        }
    }

    #[test]
    fn test_session_update_truncated_body_is_rejected() {
        for len in 0..SESSION_BYTES.len() {
            assert!(
                SChatSessionUpdate::decode(&mut &SESSION_BYTES[..len], ProtocolVersion::V1_21)
                    .is_err(),
                "a {len}-byte prefix decoded"
            );
        }
    }

    #[test]
    fn test_session_update_rejects_an_oversized_public_key() {
        let mut bytes = SESSION_BYTES[..24].to_vec();
        bytes.extend_from_slice(&[0x81, 0x04]);
        bytes.extend(std::iter::repeat_n(0u8, 513));
        bytes.push(0x00);
        assert!(SChatSessionUpdate::decode(&mut bytes.as_slice(), ProtocolVersion::V1_21).is_err());
    }
}
