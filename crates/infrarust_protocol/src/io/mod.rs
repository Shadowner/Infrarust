pub(crate) mod compression;
pub mod decoder;
pub mod encoder;
pub mod frame;

pub use decoder::PacketDecoder;
pub use encoder::PacketEncoder;
pub use frame::PacketFrame;

pub(crate) const BUFFER_RETENTION_LIMIT: usize = 64 * 1024;

pub(crate) fn release_oversized(buf: &mut Vec<u8>) {
    if buf.capacity() > BUFFER_RETENTION_LIMIT {
        *buf = Vec::new();
    }
}
