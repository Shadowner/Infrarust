//! Packet encoder — converts packets into framed bytes for the TCP socket.

use bytes::{BufMut, BytesMut};

use crate::MAX_PACKET_SIZE;
use crate::codec::VarInt;
use crate::error::{ProtocolError, ProtocolResult};
use crate::io::compression::{self, ZlibCompressor};
use crate::io::frame::PacketFrame;

/// Writes a VarInt into a BytesMut buffer.
fn write_varint(buf: &mut BytesMut, varint: VarInt) -> ProtocolResult<()> {
    varint.encode(&mut buf.writer())
}

/// Encodes packets into framed bytes ready for sending on a TCP socket.
///
/// Accumulates encoded bytes in an internal buffer. The caller retrieves
/// bytes via [`take`](Self::take) and writes them to the socket.
///
/// Encryption is **not** handled here — it is applied downstream by the
/// transport layer after `take()`.
///
/// # Example
///
/// ```
/// use infrarust_protocol::{PacketEncoder, PacketDecoder, PacketFrame};
/// use bytes::Bytes;
///
/// let mut encoder = PacketEncoder::new();
/// let mut decoder = PacketDecoder::new();
///
/// // Encode a frame
/// let frame = PacketFrame { id: 0x42, payload: Bytes::from_static(b"hello") };
/// encoder.append_frame(&frame).unwrap();
///
/// // Feed encoded bytes into the decoder
/// decoder.queue_bytes(&encoder.take());
///
/// // Decode back
/// let decoded = decoder.try_next_frame().unwrap().unwrap();
/// assert_eq!(decoded.id, 0x42);
/// assert_eq!(&decoded.payload[..], b"hello");
/// ```
pub struct PacketEncoder {
    buf: BytesMut,
    compression_threshold: Option<i32>,
    compress_buf: Vec<u8>,
    compressor: Box<dyn ZlibCompressor + Send + Sync>,
}

impl PacketEncoder {
    /// Creates a new encoder with no compression.
    pub fn new() -> Self {
        Self {
            buf: BytesMut::new(),
            compression_threshold: None,
            compress_buf: Vec::new(),
            compressor: compression::new_compressor(4),
        }
    }

    /// Encodes an opaque packet (packet_id + raw payload) into the buffer.
    ///
    /// This is the most common path for a proxy: the packet was received as a
    /// `PacketFrame` and is forwarded as-is.
    pub fn append_raw(&mut self, packet_id: i32, payload: &[u8]) -> ProtocolResult<()> {
        let packet_id_varint = VarInt(packet_id);
        let packet_id_size = packet_id_varint.written_size();

        match self.compression_threshold {
            None => {
                // [VarInt(packet_id_size + payload_len)] [VarInt(packet_id)] [payload]
                let data_len = packet_id_size + payload.len();
                if data_len > MAX_PACKET_SIZE {
                    return Err(ProtocolError::too_large(MAX_PACKET_SIZE, data_len));
                }
                let frame_len_varint = VarInt(data_len as i32);

                self.buf.reserve(frame_len_varint.written_size() + data_len);
                write_varint(&mut self.buf, frame_len_varint)?;
                write_varint(&mut self.buf, packet_id_varint)?;
                self.buf.extend_from_slice(payload);
            }
            Some(threshold) => {
                let uncompressed_len = packet_id_size + payload.len();

                if (uncompressed_len as i32) >= threshold {
                    // Compress: [VarInt(packet_len)] [VarInt(uncompressed_len)] [compressed(VarInt(packet_id) + payload)]

                    // Build uncompressed data
                    let mut uncompressed_data = Vec::with_capacity(packet_id_size + payload.len());
                    packet_id_varint.encode(&mut uncompressed_data)?;
                    uncompressed_data.extend_from_slice(payload);

                    // Compress via the abstraction
                    self.compressor
                        .compress(&uncompressed_data, &mut self.compress_buf)?;

                    let compressed_size = self.compress_buf.len();
                    let data_len_varint = VarInt(uncompressed_len as i32);
                    let packet_len = data_len_varint.written_size() + compressed_size;
                    if packet_len > MAX_PACKET_SIZE {
                        return Err(ProtocolError::too_large(MAX_PACKET_SIZE, packet_len));
                    }
                    let packet_len_varint = VarInt(packet_len as i32);

                    self.buf
                        .reserve(packet_len_varint.written_size() + packet_len);
                    write_varint(&mut self.buf, packet_len_varint)?;
                    write_varint(&mut self.buf, data_len_varint)?;
                    self.buf.extend_from_slice(&self.compress_buf);
                } else {
                    // Below threshold: [VarInt(packet_len)] [VarInt(0)] [VarInt(packet_id)] [payload]
                    let data_len_varint = VarInt(0);
                    let packet_len =
                        data_len_varint.written_size() + packet_id_size + payload.len();
                    if packet_len > MAX_PACKET_SIZE {
                        return Err(ProtocolError::too_large(MAX_PACKET_SIZE, packet_len));
                    }
                    let packet_len_varint = VarInt(packet_len as i32);

                    self.buf
                        .reserve(packet_len_varint.written_size() + packet_len);
                    write_varint(&mut self.buf, packet_len_varint)?;
                    write_varint(&mut self.buf, data_len_varint)?;
                    write_varint(&mut self.buf, packet_id_varint)?;
                    self.buf.extend_from_slice(payload);
                }
            }
        }

        Ok(())
    }

    /// Encodes a [`PacketFrame`] directly (shortcut for [`append_raw`](Self::append_raw)).
    pub fn append_frame(&mut self, frame: &PacketFrame) -> ProtocolResult<()> {
        self.append_raw(frame.id, &frame.payload)
    }

    /// Takes all accumulated encoded bytes.
    ///
    /// Returns the buffer and replaces it with an empty one.
    /// The returned bytes are ready to be sent on the socket
    /// (after encryption if needed).
    pub fn take(&mut self) -> BytesMut {
        self.buf.split()
    }

    /// Enables compression with the given threshold.
    ///
    /// Called when the proxy receives/sends a SetCompression packet.
    /// `threshold` is the minimum uncompressed size (in bytes) above which
    /// packets are compressed. Typically 256.
    pub fn set_compression(&mut self, threshold: i32) {
        self.compression_threshold = Some(threshold);
    }

    /// Returns the current compression threshold, or `None` if compression is disabled.
    pub fn compression_threshold(&self) -> Option<i32> {
        self.compression_threshold
    }
}

impl Default for PacketEncoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::decoder::PacketDecoder;

    #[test]
    fn test_encode_raw_produces_valid_frame() {
        let mut encoder = PacketEncoder::new();
        encoder.append_raw(0x00, b"hello").unwrap();
        let bytes = encoder.take();

        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x00);
        assert_eq!(&frame.payload[..], b"hello");
    }

    #[test]
    fn test_encode_frame_shortcut() {
        let frame = PacketFrame {
            id: 0x0F,
            payload: bytes::Bytes::from_static(b"test"),
        };

        let mut enc1 = PacketEncoder::new();
        enc1.append_frame(&frame).unwrap();
        let bytes1 = enc1.take();

        let mut enc2 = PacketEncoder::new();
        enc2.append_raw(0x0F, b"test").unwrap();
        let bytes2 = enc2.take();

        assert_eq!(bytes1, bytes2);
    }

    #[test]
    fn test_encode_multiple_then_take() {
        let mut encoder = PacketEncoder::new();
        encoder.append_raw(0x00, b"one").unwrap();
        encoder.append_raw(0x01, b"two").unwrap();
        encoder.append_raw(0x02, b"three").unwrap();
        let bytes = encoder.take();

        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&bytes);

        let f1 = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(f1.id, 0x00);
        assert_eq!(&f1.payload[..], b"one");

        let f2 = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(f2.id, 0x01);
        assert_eq!(&f2.payload[..], b"two");

        let f3 = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(f3.id, 0x02);
        assert_eq!(&f3.payload[..], b"three");
    }

    #[test]
    fn test_take_empties_buffer() {
        let mut encoder = PacketEncoder::new();
        encoder.append_raw(0x00, b"data").unwrap();
        let _ = encoder.take();
        let empty = encoder.take();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_encode_too_large_packet() {
        let mut encoder = PacketEncoder::new();
        let huge = vec![0u8; MAX_PACKET_SIZE + 1];
        let result = encoder.append_raw(0x00, &huge);
        assert!(result.is_err());
    }

    #[test]
    fn test_encode_with_compression_above_threshold() {
        let mut encoder = PacketEncoder::new();
        encoder.set_compression(256);

        let big_payload = vec![0x42u8; 1024];
        encoder.append_raw(0x01, &big_payload).unwrap();
        let bytes = encoder.take();

        let mut decoder = PacketDecoder::new();
        decoder.set_compression(256);
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x01);
        assert_eq!(&frame.payload[..], &big_payload[..]);
    }

    #[test]
    fn test_encode_with_compression_below_threshold() {
        let mut encoder = PacketEncoder::new();
        encoder.set_compression(256);

        encoder.append_raw(0x01, b"small").unwrap();
        let bytes = encoder.take();

        let mut decoder = PacketDecoder::new();
        decoder.set_compression(256);
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x01);
        assert_eq!(&frame.payload[..], b"small");
    }

    #[test]
    fn test_encode_decode_round_trip_no_compression() {
        let mut encoder = PacketEncoder::new();
        let payloads: &[(i32, &[u8])] = &[(0x00, b"hello"), (0x7F, b""), (0x01, &[0xFF; 100])];

        for &(id, payload) in payloads {
            encoder.append_raw(id, payload).unwrap();
        }
        let bytes = encoder.take();

        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&bytes);

        for &(id, payload) in payloads {
            let frame = decoder.try_next_frame().unwrap().unwrap();
            assert_eq!(frame.id, id);
            assert_eq!(&frame.payload[..], payload);
        }
        assert!(decoder.try_next_frame().unwrap().is_none());
    }

    #[test]
    fn test_encode_decode_round_trip_with_compression() {
        let mut encoder = PacketEncoder::new();
        let mut decoder = PacketDecoder::new();
        encoder.set_compression(64);
        decoder.set_compression(64);

        // Small packet (below threshold)
        encoder.append_raw(0x00, b"tiny").unwrap();
        // Large packet (above threshold)
        let big = vec![0xAA; 512];
        encoder.append_raw(0x01, &big).unwrap();

        let bytes = encoder.take();
        decoder.queue_bytes(&bytes);

        let f1 = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(f1.id, 0x00);
        assert_eq!(&f1.payload[..], b"tiny");

        let f2 = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(f2.id, 0x01);
        assert_eq!(&f2.payload[..], &big[..]);
    }

    #[test]
    fn test_encode_decode_round_trip_mixed() {
        let mut encoder = PacketEncoder::new();
        let mut decoder = PacketDecoder::new();

        // No compression
        encoder.append_raw(0x00, b"before compression").unwrap();
        let bytes = encoder.take();
        decoder.queue_bytes(&bytes);
        let f = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(f.id, 0x00);
        assert_eq!(&f.payload[..], b"before compression");

        // Enable compression
        encoder.set_compression(32);
        decoder.set_compression(32);

        // Below threshold
        encoder.append_raw(0x01, b"small").unwrap();
        // Above threshold
        let big = vec![0xBB; 256];
        encoder.append_raw(0x02, &big).unwrap();

        let bytes = encoder.take();
        decoder.queue_bytes(&bytes);

        let f1 = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(f1.id, 0x01);
        assert_eq!(&f1.payload[..], b"small");

        let f2 = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(f2.id, 0x02);
        assert_eq!(&f2.payload[..], &big[..]);
    }
}
