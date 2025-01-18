use bytes::BytesMut;
use std::io::{self, Write};

pub struct BytesMutWriter<'a>(pub &'a mut BytesMut);

impl Write for BytesMutWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
