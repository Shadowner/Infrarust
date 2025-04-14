use std::io::{self, Read, Write};

use crate::types::{Boolean, ProtocolRead, ProtocolString, ProtocolWrite, VarInt};
use serde::Deserialize;
use uuid::Uuid;

pub const CLIENTBOUND_LOGIN_SUCCESS_ID: i32 = 0x02;

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

impl ProtocolRead for Property {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut bytes_read = 0;

        let (name, n) = ProtocolString::read_from(reader)?;
        bytes_read += n;

        let (value, n) = ProtocolString::read_from(reader)?;
        bytes_read += n;

        let (Boolean(has_sig), n) = Boolean::read_from(reader)?;
        bytes_read += n;

        let signature = if has_sig {
            let (sig, n) = ProtocolString::read_from(reader)?;
            bytes_read += n;
            Some(sig)
        } else {
            None
        };

        Ok((
            Self {
                name,
                value,
                signature,
            },
            bytes_read,
        ))
    }
}

#[derive(Debug, Clone)]
pub struct ClientBoundLoginSuccess {
    pub uuid: Uuid,
    pub username: ProtocolString,
    pub properties: Vec<Property>,
}

impl ClientBoundLoginSuccess {
    pub fn new(uuid: Uuid, username: String, properties: Vec<Property>) -> Self {
        Self {
            uuid,
            username: ProtocolString(username),
            properties,
        }
    }
}

impl ProtocolWrite for ClientBoundLoginSuccess {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        let mut written = 0;

        // Write UUID
        writer.write_all(self.uuid.as_bytes())?;
        written += 16;

        // Write username
        written += self.username.write_to(writer)?;

        // Write properties count
        written += VarInt(self.properties.len() as i32).write_to(writer)?;

        // Write each property
        for prop in &self.properties {
            written += prop.write_to(writer)?;
        }

        Ok(written)
    }
}

impl ProtocolRead for ClientBoundLoginSuccess {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut bytes_read = 0;

        // Read UUID
        let mut uuid_bytes = [0u8; 16];
        reader.read_exact(&mut uuid_bytes)?;
        let uuid = Uuid::from_bytes(uuid_bytes);
        bytes_read += 16;

        // Read username
        let (username, n) = ProtocolString::read_from(reader)?;
        bytes_read += n;

        // Read properties count
        let (VarInt(count), n) = VarInt::read_from(reader)?;
        bytes_read += n;

        // Read each property
        let mut properties = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let (property, n) = Property::read_from(reader)?;
            bytes_read += n;
            properties.push(property);
        }

        Ok((
            Self {
                uuid,
                username,
                properties,
            },
            bytes_read,
        ))
    }
}
