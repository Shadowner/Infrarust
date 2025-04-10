use bytes::BytesMut;
use std::io::{self, Read, Write};
use tokio::io::AsyncWrite;

pub trait ProtocolWrite {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize>;
}

pub trait ProtocolRead: Sized {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)>;
}

pub trait WriteToBytes {
    fn write_to_bytes(&self, bytes: &mut BytesMut) -> io::Result<usize>;
}

#[async_trait::async_trait]
pub trait AsyncProtocolWrite: Send {
    async fn write_to_async<W: AsyncWrite + Unpin + Send>(
        &self,
        writer: &mut W,
    ) -> io::Result<usize>;
}
