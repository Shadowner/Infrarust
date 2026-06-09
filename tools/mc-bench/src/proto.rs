//! Shared protocol helpers: async framed I/O and benchmark ping constants.

use std::io;

use infrarust_protocol::version::ProtocolVersion;
use infrarust_protocol::{PacketDecoder, PacketEncoder, PacketFrame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Serverbound id for the benchmark ping. High enough that the proxy never
/// intercepts it (it falls through to opaque Play forwarding).
pub const PING_SERVERBOUND_ID: i32 = 0x40;

/// Clientbound id for the benchmark echo. Same opaque-forwarding rationale.
pub const PING_CLIENTBOUND_ID: i32 = 0x41;

/// Snaps an arbitrary protocol number to the nearest supported version that is
/// `<= proto`, so registry lookups for handshake/login packets succeed even when
/// the caller passes an exact wire number that has no dedicated constant.
///
/// The returned version is only used for registry id lookups and typed
/// encode/decode; the handshake still advertises the original `proto` number.
pub fn snap_to_supported(proto: i32) -> ProtocolVersion {
    let target = ProtocolVersion(proto);
    ProtocolVersion::SUPPORTED
        .iter()
        .copied()
        .rfind(|v| v.no_greater_than(target))
        .unwrap_or(ProtocolVersion::V1_8)
}

/// A framed connection wrapping a `TcpStream` with the protocol encoder/decoder.
pub struct FramedConn {
    stream: TcpStream,
    encoder: PacketEncoder,
    decoder: PacketDecoder,
    read_buf: Vec<u8>,
}

impl FramedConn {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            encoder: PacketEncoder::new(),
            decoder: PacketDecoder::new(),
            read_buf: vec![0u8; 8192],
        }
    }

    /// Enables compression on both directions at the given threshold.
    pub fn set_compression(&mut self, threshold: i32) {
        self.encoder.set_compression(threshold);
        self.decoder.set_compression(threshold);
    }

    /// Frames and sends a single packet, flushing the socket.
    pub async fn write_frame(&mut self, id: i32, payload: &[u8]) -> io::Result<()> {
        self.encoder
            .append_raw(id, payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let bytes = self.encoder.take();
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await
    }

    /// Reads the next complete frame, pulling from the socket as needed.
    ///
    /// Returns `Ok(None)` on a clean EOF with no buffered frame.
    pub async fn read_frame(&mut self) -> io::Result<Option<PacketFrame>> {
        loop {
            if let Some(frame) = self
                .decoder
                .try_next_frame()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?
            {
                return Ok(Some(frame));
            }
            let n = self.stream.read(&mut self.read_buf).await?;
            if n == 0 {
                return Ok(None);
            }
            self.decoder.queue_bytes(&self.read_buf[..n]);
        }
    }

    /// Splits off the read half's decoder + socket reader and the write half's
    /// encoder + socket writer so sends and receives can run concurrently.
    pub fn into_split(self) -> (FramedReader, FramedWriter) {
        let (read_half, write_half) = self.stream.into_split();
        (
            FramedReader {
                read_half,
                decoder: self.decoder,
                read_buf: self.read_buf,
            },
            FramedWriter {
                write_half,
                encoder: self.encoder,
            },
        )
    }
}

/// Read half of a split [`FramedConn`].
pub struct FramedReader {
    read_half: tokio::net::tcp::OwnedReadHalf,
    decoder: PacketDecoder,
    read_buf: Vec<u8>,
}

impl FramedReader {
    pub async fn read_frame(&mut self) -> io::Result<Option<PacketFrame>> {
        loop {
            if let Some(frame) = self
                .decoder
                .try_next_frame()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?
            {
                return Ok(Some(frame));
            }
            let n = self.read_half.read(&mut self.read_buf).await?;
            if n == 0 {
                return Ok(None);
            }
            self.decoder.queue_bytes(&self.read_buf[..n]);
        }
    }
}

/// Write half of a split [`FramedConn`].
pub struct FramedWriter {
    write_half: tokio::net::tcp::OwnedWriteHalf,
    encoder: PacketEncoder,
}

impl FramedWriter {
    pub async fn write_frame(&mut self, id: i32, payload: &[u8]) -> io::Result<()> {
        self.encoder
            .append_raw(id, payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let bytes = self.encoder.take();
        self.write_half.write_all(&bytes).await?;
        self.write_half.flush().await
    }
}
