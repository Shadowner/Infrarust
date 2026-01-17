mod buffer_pool;
mod reader;
mod utils;
mod writer;

pub use buffer_pool::{BufferPool, PooledBuffer, get_buffer, get_buffer_with_capacity, return_buffer};
pub use reader::PacketReader;
pub use writer::PacketWriter;

use super::base::Packet;
use super::error::PacketResult;
use async_trait::async_trait;
use bytes::BytesMut;

#[derive(Debug)]
pub enum PacketOrBytes {
    Packet(Packet),
    Raw(BytesMut),
}

#[async_trait]
pub trait PacketReadWrite: Send {
    async fn read_packet(&mut self) -> PacketResult<Packet>;
    async fn write_packet(&mut self, packet: &Packet) -> PacketResult<()>;
}

#[async_trait]
pub trait RawPacketIO: Send {
    async fn read_raw(&mut self) -> PacketResult<Option<BytesMut>>;
    async fn write_raw(&mut self, data: &[u8]) -> PacketResult<()>;
}

pub trait RawPacketReadWrite: RawPacketIO + PacketReadWrite {}
impl<T> RawPacketReadWrite for T where T: RawPacketIO + PacketReadWrite {}
