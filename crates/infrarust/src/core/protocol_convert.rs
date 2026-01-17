use infrarust_protocol::{
    PacketCodec, PacketDataAccess, ProtocolRead,
    minecraft::java::login::{
        CLIENTBOUND_ENCRYPTION_REQUEST_ID, ClientBoundEncryptionRequest,
        SERVERBOUND_ENCRYPTION_RESPONSE_ID, SERVERBOUND_LOGIN_START_ID,
        ServerBoundEncryptionResponse, ServerBoundLoginStart,
        clientbound_loginsuccess::{ClientBoundLoginSuccess, Property},
    },
    types::{Boolean, ByteArray, ProtocolString, ProtocolUUID, VarInt},
    version::Version,
};
use std::io::{self, Read};
use tracing::debug;
use uuid::Uuid;

use crate::network::packet::Packet;

impl From<&ClientBoundEncryptionRequest> for Packet {
    fn from(req: &ClientBoundEncryptionRequest) -> Self {
        let mut packet = Packet::new(CLIENTBOUND_ENCRYPTION_REQUEST_ID);
        packet
            .encode(&req.server_id)
            .expect("Failed to encode server_id in encryption request");
        packet
            .encode(&req.public_key)
            .expect("Failed to encode public_key in encryption request");
        packet
            .encode(&req.verify_token)
            .expect("Failed to encode verify_token in encryption request");
        packet
            .encode(&req.requires_authentication)
            .expect("Failed to encode requires_authentication in encryption request");
        packet
    }
}

impl TryFrom<&Packet> for ClientBoundEncryptionRequest {
    type Error = io::Error;

    fn try_from(packet: &Packet) -> Result<Self, Self::Error> {
        let mut cursor = io::Cursor::new(&packet.data);

        let (server_id, _) = ProtocolString::read_from(&mut cursor)?;
        let (public_key, _) = ByteArray::read_from(&mut cursor)?;
        let (verify_token, _) = ByteArray::read_from(&mut cursor)?;
        let (requires_authentication, _) = Boolean::read_from(&mut cursor)?;

        Ok(Self {
            server_id,
            public_key,
            verify_token,
            requires_authentication,
        })
    }
}

impl From<&ClientBoundLoginSuccess> for Packet {
    fn from(login: &ClientBoundLoginSuccess) -> Self {
        // Create new packet with ID 0x02 (Login Success)
        let mut packet = Packet::new(0x02);

        packet.data.extend_from_slice(login.uuid.as_bytes());
        packet
            .encode(&login.username)
            .expect("Failed to encode username in login success");
        packet
            .encode(&VarInt(login.properties.len() as i32))
            .expect("Failed to encode properties count in login success");

        // Write each property

        //TODO: Check if I've implemented this correctly
        // for prop in login.properties.clone() {
        //     packet.encode(&prop.name).unwrap();
        //     packet.encode(&prop.value).unwrap();
        //     packet.encode(&Boolean(prop.signature.is_some())).unwrap();
        //     if let Some(sig) = prop.signature {
        //         packet.encode(&sig).unwrap();
        //     }
        // }

        packet
    }
}

impl TryFrom<&Packet> for ClientBoundLoginSuccess {
    type Error = io::Error;

    fn try_from(packet: &Packet) -> Result<Self, Self::Error> {
        let mut cursor = io::Cursor::new(&packet.data);
        debug!(
            "Parsing login success packet of length {}",
            packet.data.len()
        );

        // Read UUID
        let mut uuid_bytes = [0u8; 16];
        cursor.read_exact(&mut uuid_bytes)?;
        let uuid = Uuid::from_bytes(uuid_bytes);
        debug!("Read UUID: {:?}", uuid);

        // Read username
        let (username, _) = ProtocolString::read_from(&mut cursor)?;
        debug!("Read username: {}", username.0);

        // Read properties count
        let (VarInt(count), _) = VarInt::read_from(&mut cursor)?;
        debug!("Reading {} properties", count);

        let mut properties = Vec::with_capacity(count as usize);
        for i in 0..count {
            let (name, _) = ProtocolString::read_from(&mut cursor)?;
            let (value, _) = ProtocolString::read_from(&mut cursor)?;
            let (Boolean(has_sig), _) = Boolean::read_from(&mut cursor)?;

            let signature = if has_sig {
                let (sig, _) = ProtocolString::read_from(&mut cursor)?;
                Some(sig)
            } else {
                None
            };

            debug!(
                "Read property {}: name={}, value={}, has_sig={}",
                i, name.0, value.0, has_sig
            );

            properties.push(Property {
                name,
                value,
                signature,
            });
        }

        Ok(Self {
            uuid,
            username,
            properties,
        })
    }
}
impl TryFrom<&Packet> for ServerBoundEncryptionResponse {
    type Error = io::Error;

    fn try_from(packet: &Packet) -> Result<Self, Self::Error> {
        if packet.id != SERVERBOUND_ENCRYPTION_RESPONSE_ID {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid packet ID for encryption response",
            ));
        }
        packet
            .decode()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
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

        packet
            .encode(&login.name)
            .expect("Failed to encode name in login start");
        if let Some(uuid) = &login.player_uuid {
            packet
                .encode(uuid)
                .expect("Failed to encode player_uuid in login start");
        }
        packet.set_protocol_version(login.protocol_version);
        packet
    }
}
