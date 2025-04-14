use crate::types::{ProtocolRead, ProtocolWrite, VarInt};
use std::io;

pub const CLIENTBOUND_SET_COMPRESSION_ID: i32 = 0x03;
pub const DEFAULT_COMPRESSION_THRESHOLD: i32 = 256; // Default compression threshold

#[derive(Debug, Clone)]
pub struct ClientBoundSetCompression {
    pub threshold: VarInt,
}

impl ClientBoundSetCompression {
    pub fn new(threshold: i32) -> Self {
        Self {
            threshold: VarInt(threshold),
        }
    }
}

impl Default for ClientBoundSetCompression {
    fn default() -> Self {
        Self::new(DEFAULT_COMPRESSION_THRESHOLD)
    }
}

impl ProtocolWrite for ClientBoundSetCompression {
    fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        self.threshold.write_to(writer)
    }
}

impl ProtocolRead for ClientBoundSetCompression {
    fn read_from<R: io::Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let (threshold, n) = VarInt::read_from(reader)?;
        Ok((Self { threshold }, n))
    }
}
