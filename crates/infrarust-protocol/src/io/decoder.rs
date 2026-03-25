//! Packet decoder — converts a TCP byte stream into individual `PacketFrame`s.

use bytes::{Buf, BytesMut};

use crate::codec::VarInt;
use crate::codec::varint::VarIntDecodeStatus;
use crate::error::{ProtocolError, ProtocolResult};
use crate::io::compression::{self, ZlibDecompressor};
use crate::io::frame::PacketFrame;
use crate::{MAX_PACKET_DATA_SIZE, MAX_PACKET_SIZE};

/// Decodes a TCP byte stream into individual [`PacketFrame`]s.
///
/// The Minecraft protocol frames each packet as:
///
/// **Without compression:**
/// ```text
/// [Packet Length: VarInt] [Packet ID: VarInt] [Payload: bytes]
/// ```
///
/// **With compression** (after SetCompression):
/// ```text
/// [Packet Length: VarInt] [Data Length: VarInt] [Compressed(Packet ID + Payload): bytes]
/// ```
/// If `Data Length == 0`, the content is not compressed (size < threshold).
///
/// **Encryption is NOT handled here.** The proxy manages two independent
/// crypto tunnels (client↔proxy and proxy↔backend). Encryption/decryption
/// is applied upstream by the transport layer before feeding the decoder.
pub struct PacketDecoder {
    buf: BytesMut,
    compression_threshold: Option<i32>,
    decompressor: Box<dyn ZlibDecompressor + Send + Sync>,
    decompress_buf: Vec<u8>,
}

impl PacketDecoder {
    pub fn new() -> Self {
        Self {
            buf: BytesMut::new(),
            compression_threshold: None,
            decompressor: compression::new_decompressor(),
            decompress_buf: Vec::new(),
        }
    }

    /// Appends bytes received from the socket to the internal buffer.
    ///
    /// The bytes are typically the result of a `TcpStream::read()`.
    /// If encryption is active, bytes must have been decrypted BEFORE this call.
    pub fn queue_bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Attempts to extract the next complete [`PacketFrame`] from the buffer.
    ///
    /// Returns:
    /// - `Ok(Some(frame))` — a complete frame was extracted
    /// - `Ok(None)` — not enough data yet, wait for more bytes
    /// - `Err(...)` — corrupted data or packet too large
    ///
    /// Can be called in a loop until `Ok(None)` to extract all available frames.
    ///
    /// # Errors
    /// Returns an error if the packet data is corrupted, exceeds size limits,
    /// or decompression fails.
    pub fn try_next_frame(&mut self) -> ProtocolResult<Option<PacketFrame>> {
        // 1. Try to read VarInt(packet_len) without consuming
        let (packet_len_varint, varint_size) = match VarInt::decode_partial(&self.buf) {
            Ok(result) => result,
            Err(VarIntDecodeStatus::Incomplete) => return Ok(None),
            Err(VarIntDecodeStatus::TooLarge) => {
                return Err(ProtocolError::invalid("packet length VarInt too large"));
            }
        };

        let packet_len = packet_len_varint.0;

        // 2. Validate packet_len
        if packet_len <= 0 {
            return Err(ProtocolError::invalid("packet length must be positive"));
        }
        let packet_len = packet_len as usize;
        if packet_len > MAX_PACKET_SIZE {
            return Err(ProtocolError::too_large(MAX_PACKET_SIZE, packet_len));
        }

        // 3. Check if we have enough data
        if self.buf.len() < varint_size + packet_len {
            return Ok(None);
        }

        // 4. Consume the length VarInt and split packet data
        self.buf.advance(varint_size);
        let mut data = self.buf.split_to(packet_len);

        // 5. Decode based on compression mode
        #[allow(clippy::branches_sharing_code)]
        // Both branches share initial cursor setup but diverge significantly after
        if self.compression_threshold.is_none() {
            // No compression: [VarInt(packet_id)] [payload]
            let slice = &data[..];
            let mut cursor = slice;
            let packet_id = VarInt::decode(&mut cursor)?;
            let id_size = slice.len() - cursor.len();
            data.advance(id_size);
            Ok(Some(PacketFrame {
                id: packet_id.0,
                payload: data.freeze(),
            }))
        } else {
            // Compression mode: [VarInt(data_len)] [compressed or raw data]
            let slice = &data[..];
            let mut cursor = slice;
            let data_len = VarInt::decode(&mut cursor)?;
            let data_len_varint_size = slice.len() - cursor.len();
            data.advance(data_len_varint_size);

            if data_len.0 == 0 {
                // Not compressed (below threshold)
                let slice = &data[..];
                let mut cursor = slice;
                let packet_id = VarInt::decode(&mut cursor)?;
                let id_size = slice.len() - cursor.len();
                data.advance(id_size);
                Ok(Some(PacketFrame {
                    id: packet_id.0,
                    payload: data.freeze(),
                }))
            } else {
                // Compressed
                let data_len = data_len.0 as usize;
                if data_len > MAX_PACKET_DATA_SIZE {
                    return Err(ProtocolError::too_large(MAX_PACKET_DATA_SIZE, data_len));
                }

                self.decompressor
                    .decompress(&data[..], &mut self.decompress_buf, data_len)?;

                let mut cursor: &[u8] = &self.decompress_buf;
                let packet_id = VarInt::decode(&mut cursor)?;
                let payload = bytes::Bytes::copy_from_slice(cursor);
                Ok(Some(PacketFrame {
                    id: packet_id.0,
                    payload,
                }))
            }
        }
    }

    /// Enables compression with the given threshold.
    ///
    /// Called when the proxy receives/sends a `SetCompression` packet.
    /// After this call, all packets are expected in compressed format.
    /// `threshold` is the minimum uncompressed size (in bytes) above which
    /// packets are compressed. Typically 256.
    pub const fn set_compression(&mut self, threshold: i32) {
        self.compression_threshold = Some(threshold);
    }

    pub const fn compression_threshold(&self) -> Option<i32> {
        self.compression_threshold
    }

    pub fn into_remaining(self) -> BytesMut {
        self.buf
    }
}

impl Default for PacketDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::io::encoder::PacketEncoder;

    /// Manually encode a simple uncompressed frame.
    fn encode_frame(packet_id: i32, payload: &[u8]) -> Vec<u8> {
        let id_varint = VarInt(packet_id);
        let data_len = id_varint.written_size() + payload.len();
        let len_varint = VarInt(data_len as i32);

        let mut buf = Vec::new();
        len_varint.encode(&mut buf).unwrap();
        id_varint.encode(&mut buf).unwrap();
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn test_decode_single_frame() {
        let raw = encode_frame(0x00, b"hello");
        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&raw);

        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x00);
        assert_eq!(&frame.payload[..], b"hello");
    }

    #[test]
    fn test_decode_fragmented_arrival() {
        let raw = encode_frame(0x01, b"world");
        let mut decoder = PacketDecoder::new();

        // Feed first half
        let mid = raw.len() / 2;
        decoder.queue_bytes(&raw[..mid]);
        assert!(decoder.try_next_frame().unwrap().is_none());

        // Feed second half
        decoder.queue_bytes(&raw[mid..]);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x01);
        assert_eq!(&frame.payload[..], b"world");
    }

    #[test]
    fn test_decode_multiple_frames_in_one_buffer() {
        let mut raw = encode_frame(0x00, b"one");
        raw.extend_from_slice(&encode_frame(0x01, b"two"));

        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&raw);

        let f1 = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(f1.id, 0x00);
        assert_eq!(&f1.payload[..], b"one");

        let f2 = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(f2.id, 0x01);
        assert_eq!(&f2.payload[..], b"two");

        assert!(decoder.try_next_frame().unwrap().is_none());
    }

    #[test]
    fn test_decode_empty_buffer() {
        let mut decoder = PacketDecoder::new();
        assert!(decoder.try_next_frame().unwrap().is_none());
    }

    #[test]
    fn test_decode_incomplete_varint() {
        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&[0x80]); // continuation bit set, need more bytes
        assert!(decoder.try_next_frame().unwrap().is_none());
    }

    #[test]
    fn test_decode_packet_too_large() {
        // Encode a VarInt with value > MAX_PACKET_SIZE
        let big_len = VarInt((MAX_PACKET_SIZE + 1) as i32);
        let mut buf = Vec::new();
        big_len.encode(&mut buf).unwrap();
        // Don't need actual data — should fail on length check

        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&buf);
        let err = decoder.try_next_frame().unwrap_err();
        assert!(matches!(err, ProtocolError::TooLarge { .. }));
    }

    #[test]
    fn test_decode_zero_length_packet() {
        let mut buf = Vec::new();
        VarInt(0).encode(&mut buf).unwrap();

        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&buf);
        let err = decoder.try_next_frame().unwrap_err();
        assert!(matches!(err, ProtocolError::Invalid { .. }));
    }

    #[test]
    fn test_decode_with_compression_uncompressed() {
        // Use encoder to produce a correctly formatted compressed-mode packet below threshold
        let mut encoder = PacketEncoder::new();
        encoder.set_compression(256);
        encoder.append_raw(0x05, b"small").unwrap();
        let bytes = encoder.take();

        let mut decoder = PacketDecoder::new();
        decoder.set_compression(256);
        decoder.queue_bytes(&bytes);

        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x05);
        assert_eq!(&frame.payload[..], b"small");
    }

    #[test]
    fn test_decode_with_compression_compressed() {
        let mut encoder = PacketEncoder::new();
        encoder.set_compression(64);
        let big_payload = vec![0x42; 512];
        encoder.append_raw(0x03, &big_payload).unwrap();
        let bytes = encoder.take();

        let mut decoder = PacketDecoder::new();
        decoder.set_compression(64);
        decoder.queue_bytes(&bytes);

        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x03);
        assert_eq!(&frame.payload[..], &big_payload[..]);
    }

    #[test]
    fn test_decode_compressed_zip_bomb_protection() {
        // Manually craft a packet with data_len > MAX_PACKET_DATA_SIZE
        // Format: [VarInt(packet_len)] [VarInt(huge_data_len)] [some bytes]
        let huge_data_len = VarInt((MAX_PACKET_DATA_SIZE + 1) as i32);
        let mut inner = Vec::new();
        huge_data_len.encode(&mut inner).unwrap();
        inner.extend_from_slice(&[0x00; 10]); // some garbage bytes

        let packet_len = VarInt(inner.len() as i32);
        let mut buf = Vec::new();
        packet_len.encode(&mut buf).unwrap();
        buf.extend_from_slice(&inner);

        let mut decoder = PacketDecoder::new();
        decoder.set_compression(1);
        decoder.queue_bytes(&buf);

        let err = decoder.try_next_frame().unwrap_err();
        assert!(matches!(err, ProtocolError::TooLarge { .. }));
    }

    #[test]
    fn test_decode_corrupted_compressed_data() {
        // Craft a packet claiming data_len > 0 but with garbage compressed data
        let data_len = VarInt(100);
        let mut inner = Vec::new();
        data_len.encode(&mut inner).unwrap();
        inner.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]); // garbage

        let packet_len = VarInt(inner.len() as i32);
        let mut buf = Vec::new();
        packet_len.encode(&mut buf).unwrap();
        buf.extend_from_slice(&inner);

        let mut decoder = PacketDecoder::new();
        decoder.set_compression(1);
        decoder.queue_bytes(&buf);

        let err = decoder.try_next_frame().unwrap_err();
        assert!(matches!(err, ProtocolError::Invalid { .. }));
    }

    #[test]
    fn test_decode_compression_round_trip_with_encoder() {
        let mut encoder = PacketEncoder::new();
        let mut decoder = PacketDecoder::new();
        encoder.set_compression(128);
        decoder.set_compression(128);

        let payloads: Vec<(i32, Vec<u8>)> = vec![
            (0x00, b"tiny".to_vec()),
            (0x01, vec![0xAA; 256]),
            (0x02, vec![0xBB; 1024]),
            (0x03, b"also small".to_vec()),
        ];

        for (id, payload) in &payloads {
            encoder.append_raw(*id, payload).unwrap();
        }
        let bytes = encoder.take();
        decoder.queue_bytes(&bytes);

        for (id, payload) in &payloads {
            let frame = decoder.try_next_frame().unwrap().unwrap();
            assert_eq!(frame.id, *id);
            assert_eq!(&frame.payload[..], &payload[..]);
        }
        assert!(decoder.try_next_frame().unwrap().is_none());
    }

    // Integration tests

    #[test]
    fn test_full_pipeline_encode_decode_with_compression() {
        let mut encoder = PacketEncoder::new();
        let mut decoder = PacketDecoder::new();

        // Without compression
        encoder.append_raw(0x00, b"hello").unwrap();
        let bytes = encoder.take();
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x00);
        assert_eq!(&frame.payload[..], b"hello");

        // Enable compression
        encoder.set_compression(256);
        decoder.set_compression(256);

        // Small packet (< threshold)
        encoder.append_raw(0x01, b"small").unwrap();
        let bytes = encoder.take();
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x01);
        assert_eq!(&frame.payload[..], b"small");

        // Big packet (> threshold)
        let big_payload = vec![0x42; 1024];
        encoder.append_raw(0x02, &big_payload).unwrap();
        let bytes = encoder.take();
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x02);
        assert_eq!(&frame.payload[..], &big_payload[..]);
    }

    #[test]
    fn test_full_pipeline_with_encryption() {
        use crate::crypto::{DecryptCipher, EncryptCipher};

        let key = [0x42u8; 16];

        let mut encoder = PacketEncoder::new();
        encoder.append_raw(0x00, b"secret data").unwrap();
        let mut bytes = encoder.take();

        let mut encrypt = EncryptCipher::new(&key);
        encrypt.encrypt(&mut bytes);

        let mut decrypt = DecryptCipher::new(&key);
        decrypt.decrypt(&mut bytes);

        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x00);
        assert_eq!(&frame.payload[..], b"secret data");
    }

    /// Compile-time assertion that types are Send + Sync.
    #[test]
    fn test_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PacketDecoder>();
        assert_send_sync::<PacketEncoder>();
        assert_send_sync::<crate::crypto::EncryptCipher>();
        assert_send_sync::<crate::crypto::DecryptCipher>();
    }
}
