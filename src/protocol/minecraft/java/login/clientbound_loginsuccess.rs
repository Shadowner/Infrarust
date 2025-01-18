use std::io::{self, Read, Write};

use crate::network::packet::{Packet, PacketCodec};
use crate::protocol::types::{Boolean, ProtocolRead, ProtocolString, ProtocolWrite, VarInt};
use log::debug;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize, Clone)]
pub struct Property {
    pub name: ProtocolString,
    pub value: ProtocolString,
    pub signature: Option<ProtocolString>,
}

impl ProtocolWrite for Property {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        let mut written = 0;
        written += self.name.write_to(writer)?;
        written += self.value.write_to(writer)?;
        written += Boolean(self.signature.is_some()).write_to(writer)?;
        if let Some(ref sig) = self.signature {
            written += sig.write_to(writer)?;
        }
        Ok(written)
    }
}

#[derive(Debug, Clone)]
pub struct ClientBoundLoginSuccess {
    pub uuid: Uuid,
    pub username: ProtocolString,
    pub properties: Vec<Property>,
}

impl From<&ClientBoundLoginSuccess> for Packet {
    fn from(login: &ClientBoundLoginSuccess) -> Self {
        // Create new packet with ID 0x02 (Login Success)
        let mut packet = Packet::new(0x02);

        packet.data.extend_from_slice(login.uuid.as_bytes());
        packet.encode(&login.username).unwrap();
        packet
            .encode(&VarInt(login.properties.len() as i32))
            .unwrap();

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
