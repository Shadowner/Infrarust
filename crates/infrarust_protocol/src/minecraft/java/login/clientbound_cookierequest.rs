use crate::types::{Identifier, ProtocolRead, ProtocolWrite};
use std::io;

pub const CLIENTBOUND_COOKIE_REQUEST_ID: i32 = 0x05;

#[derive(Debug, Clone)]
pub struct ClientBoundCookieRequest {
    pub key: Identifier,
}

impl ClientBoundCookieRequest {
    pub fn new(key: Identifier) -> Self {
        Self { key }
    }
}

impl ProtocolWrite for ClientBoundCookieRequest {
    fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        self.key.write_to(writer)
    }
}

impl ProtocolRead for ClientBoundCookieRequest {
    fn read_from<R: io::Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let (key, n) = Identifier::read_from(reader)?;
        Ok((Self { key }, n))
    }
}
