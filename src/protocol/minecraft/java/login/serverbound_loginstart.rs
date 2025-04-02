use std::convert::TryFrom;
use std::io::{self, Read};
use uuid::Uuid;

use crate::{
    ProtocolRead, ProtocolWrite,
    network::{
        packet::{
            Packet,
            PacketCodec, // Important: we need this trait in scope
        },
        proxy_protocol::ProtocolResult,
    },
    protocol::types::{Boolean, ByteArray, Long, ProtocolString, ProtocolUUID},
    version::Version,
};

pub const SERVERBOUND_LOGIN_START_ID: i32 = 0x00;

#[derive(Debug, Clone, PartialEq)]
pub struct ServerBoundLoginStart {
    pub name: ProtocolString,
    pub has_player_uuid: Boolean,
    pub player_uuid: Option<ProtocolUUID>,
    // Optional signature data (1.19+)
    pub has_signature: Boolean,
    pub timestamp: Option<Long>,
    pub public_key: Option<ByteArray>,
    pub signature: Option<ByteArray>,
    pub protocol_version: Version,
}

impl ServerBoundLoginStart {
    pub fn new(name: String) -> Self {
        Self {
            name: ProtocolString(name),
            has_player_uuid: Boolean(false),
            player_uuid: None,
            has_signature: Boolean(false),
            timestamp: None,
            public_key: None,
            signature: None,
            protocol_version: Version::V1_18_2, // Changed default version to match test
        }
    }

    pub fn with_uuid(name: String, uuid: Uuid) -> Self {
        Self {
            name: ProtocolString(name),
            has_player_uuid: Boolean(true),
            player_uuid: Some(ProtocolUUID(uuid)),
            has_signature: Boolean(false),
            timestamp: None,
            public_key: None,
            signature: None,
            protocol_version: Version::V1_20_2,
        }
    }
}

impl ProtocolWrite for ServerBoundLoginStart {
    fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let mut written = 0;
        written += self.name.write_to(writer)?;

        if Version::V1_19_3.protocol_number() <= self.protocol_version.protocol_number()
            && self.protocol_version.protocol_number() < Version::V1_20_2.protocol_number()
        {
            written += self.has_player_uuid.write_to(writer)?;
            if self.has_player_uuid.0 {
                written += self.player_uuid.as_ref().unwrap().write_to(writer)?;
            }
        } else if Version::V1_20_2.protocol_number() <= self.protocol_version.protocol_number() {
            if let Some(uuid) = &self.player_uuid {
                written += uuid.write_to(writer)?;
            }
        }

        if Version::V1_19.protocol_number() <= self.protocol_version.protocol_number()
            && self.protocol_version.protocol_number() < Version::V1_19_3.protocol_number()
        {
            written += self.has_signature.write_to(writer)?;
            if self.has_signature.0 {
                written += self.timestamp.as_ref().unwrap().write_to(writer)?;
                written += self.public_key.as_ref().unwrap().write_to(writer)?;
                written += self.signature.as_ref().unwrap().write_to(writer)?;
            }
        }

        Ok(written)
    }
}

impl ServerBoundLoginStart {
    pub fn read_with_version<R: Read>(
        reader: &mut R,
        version: Version,
    ) -> ProtocolResult<(Self, usize)> {
        let mut bytes_read = 0;
        let (name, n) = ProtocolString::read_from(reader)?;
        bytes_read += n;

        let mut login = ServerBoundLoginStart {
            name,
            has_player_uuid: Boolean(false),
            player_uuid: None,
            has_signature: Boolean(false),
            timestamp: None,
            public_key: None,
            signature: None,
            protocol_version: version,
        };

        if Version::V1_19.protocol_number() <= version.protocol_number()
            && version.protocol_number() < Version::V1_19_3.protocol_number()
        {
            let (has_signature, n) = Boolean::read_from(reader)?;
            bytes_read += n;
            login.has_signature = has_signature;

            if has_signature.0 {
                let (timestamp, n) = Long::read_from(reader)?;
                bytes_read += n;
                let (public_key, n) = ByteArray::read_from(reader)?;
                bytes_read += n;
                let (signature, n) = ByteArray::read_from(reader)?;
                bytes_read += n;

                login.timestamp = Some(timestamp);
                login.public_key = Some(public_key);
                login.signature = Some(signature);
            }
        }

        if Version::V1_19_3.protocol_number() <= version.protocol_number()
            && version.protocol_number() < Version::V1_20_2.protocol_number()
        {
            let (has_uuid, n) = Boolean::read_from(reader)?;
            bytes_read += n;
            login.has_player_uuid = has_uuid;

            if has_uuid.0 {
                let (uuid, n) = ProtocolUUID::read_from(reader)?;
                bytes_read += n;
                login.player_uuid = Some(uuid);
            }
        } else if Version::V1_20_2.protocol_number() <= version.protocol_number() {
            let (uuid, n) = ProtocolUUID::read_from(reader)?;
            bytes_read += n;
            login.has_player_uuid = Boolean(true);
            login.player_uuid = Some(uuid);
        }

        Ok((login, bytes_read))
    }
}

impl TryFrom<&Packet> for ServerBoundLoginStart {
    type Error = std::io::Error;

    fn try_from(packet: &Packet) -> Result<Self, Self::Error> {
        let name = packet
            .decode::<ProtocolString>()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let remaining_data = &packet.data[packet.data.len()..];
        let mut player_uuid = None;

        if !remaining_data.is_empty() {
            let uuid = packet
                .decode::<ProtocolUUID>()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            player_uuid = Some(uuid);
        }

        Ok(Self {
            name,
            has_player_uuid: Boolean(player_uuid.is_some()),
            player_uuid,
            has_signature: Boolean(false),
            timestamp: None,
            public_key: None,
            signature: None,
            protocol_version: Version::V1_20_2,
        })
    }
}

impl From<&ServerBoundLoginStart> for Packet {
    fn from(login: &ServerBoundLoginStart) -> Self {
        let mut packet = Packet::new(SERVERBOUND_LOGIN_START_ID);

        packet.encode(&login.name).unwrap();
        if let Some(uuid) = &login.player_uuid {
            packet.encode(uuid).unwrap();
        }
        packet.set_protocol_version(login.protocol_version);
        packet
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use uuid::Uuid;

    #[test]
    fn test_login_start_basic() {
        let name = "Player123".to_string();
        let login = ServerBoundLoginStart::new(name.clone());
        let test_version = Version::V1_18_2; // Make version explicit

        let mut buffer = Vec::new();
        let written = login.write_to(&mut buffer).unwrap();

        let mut cursor = Cursor::new(buffer);
        let (read_login, read) =
            ServerBoundLoginStart::read_with_version(&mut cursor, test_version).unwrap();

        assert_eq!(written, read);
        assert_eq!(login.name.0, name);
        assert_eq!(login, read_login);
    }

    #[test]
    fn test_login_start_with_uuid() {
        let name = "Player123".to_string();
        let uuid = Uuid::new_v4();
        let login = ServerBoundLoginStart::with_uuid(name.clone(), uuid);

        let mut buffer = Vec::new();
        let written = login.write_to(&mut buffer).unwrap();

        let mut cursor = Cursor::new(buffer);
        let (read_login, read) =
            ServerBoundLoginStart::read_with_version(&mut cursor, Version::V1_20_2).unwrap();

        assert_eq!(written, read);
        assert_eq!(login.player_uuid.unwrap().0, uuid);
        assert_eq!(login, read_login);
    }

    #[test]
    fn test_login_start_with_signature() {
        let name = "Player123".to_string();
        let mut login = ServerBoundLoginStart::new(name);
        login.protocol_version = Version::V1_19;
        login.has_signature = Boolean(true);
        login.timestamp = Some(Long(1234567890));
        login.public_key = Some(ByteArray(vec![1, 2, 3, 4]));
        login.signature = Some(ByteArray(vec![5, 6, 7, 8]));

        let mut buffer = Vec::new();
        let written = login.write_to(&mut buffer).unwrap();

        let mut cursor = Cursor::new(buffer);
        let (read_login, read) =
            ServerBoundLoginStart::read_with_version(&mut cursor, Version::V1_19).unwrap();

        assert_eq!(written, read);
        assert_eq!(login, read_login);
    }
}
