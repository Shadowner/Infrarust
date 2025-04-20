use crate::types::{ByteArray, Identifier, ProtocolRead, ProtocolWrite, VarInt};
use std::io;

pub const CLIENTBOUND_PLUGIN_REQUEST_ID: i32 = 0x04;

#[derive(Debug, Clone)]
pub struct ClientBoundPluginRequest {
    pub message_id: VarInt,
    pub channel: Identifier,
    pub data: ByteArray,
}

impl ClientBoundPluginRequest {
    pub fn new(message_id: i32, channel: Identifier, data: Vec<u8>) -> Self {
        Self {
            message_id: VarInt(message_id),
            channel,
            data: ByteArray(data),
        }
    }
}

impl ProtocolWrite for ClientBoundPluginRequest {
    fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let mut written = 0;
        written += self.message_id.write_to(writer)?;
        written += self.channel.write_to(writer)?;
        written += self.data.write_to(writer)?;
        Ok(written)
    }
}

impl ProtocolRead for ClientBoundPluginRequest {
    fn read_from<R: io::Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut bytes_read = 0;

        let (message_id, n) = VarInt::read_from(reader)?;
        bytes_read += n;

        let (channel, n) = Identifier::read_from(reader)?;
        bytes_read += n;

        let (data, n) = ByteArray::read_from(reader)?;
        bytes_read += n;

        Ok((
            Self {
                message_id,
                channel,
                data,
            },
            bytes_read,
        ))
    }
}
