use bytes::BytesMut;

use crate::codec::VarInt;
use crate::codec::varint::VarIntDecodeStatus;
use crate::error::{ProtocolError, ProtocolResult};
use crate::io::compression::{self, ZlibDecompressor};
use crate::io::frame::PacketFrame;
use crate::{MAX_PACKET_DATA_SIZE, MAX_PACKET_SIZE};

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

    pub fn queue_bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn read_buf_mut(&mut self) -> &mut BytesMut {
        &mut self.buf
    }

    pub fn try_next_frame(&mut self) -> ProtocolResult<Option<PacketFrame>> {
        let (packet_len_varint, varint_size) = match VarInt::decode_partial(&self.buf) {
            Ok(result) => result,
            Err(VarIntDecodeStatus::Incomplete) => return Ok(None),
            Err(VarIntDecodeStatus::TooLarge) => {
                return Err(ProtocolError::invalid("packet length VarInt too large"));
            }
        };

        let packet_len = packet_len_varint.0;

        if packet_len <= 0 {
            return Err(ProtocolError::invalid("packet length must be positive"));
        }
        let packet_len = packet_len as usize;
        if packet_len > MAX_PACKET_SIZE {
            return Err(ProtocolError::too_large(MAX_PACKET_SIZE, packet_len));
        }

        if self.buf.len() < varint_size + packet_len {
            return Ok(None);
        }

        let wire = self.buf.split_to(varint_size + packet_len).freeze();
        let data = wire.slice(varint_size..);

        if self.compression_threshold.is_none() {
            let mut cursor = &data[..];
            let packet_id = VarInt::decode(&mut cursor)?;
            let id_size = data.len() - cursor.len();
            let payload = data.slice(id_size..);
            Ok(Some(PacketFrame::with_raw(
                packet_id.0,
                payload,
                wire,
                None,
            )))
        } else {
            let mut cursor = &data[..];
            let data_len = VarInt::decode(&mut cursor)?;
            let data_len_varint_size = data.len() - cursor.len();
            let data = data.slice(data_len_varint_size..);

            if data_len.0 == 0 {
                let mut cursor = &data[..];
                let packet_id = VarInt::decode(&mut cursor)?;
                let id_size = data.len() - cursor.len();
                let payload = data.slice(id_size..);
                Ok(Some(PacketFrame::with_raw(
                    packet_id.0,
                    payload,
                    wire,
                    self.compression_threshold,
                )))
            } else {
                let data_len = data_len.0 as usize;
                if data_len > MAX_PACKET_DATA_SIZE {
                    return Err(ProtocolError::too_large(MAX_PACKET_DATA_SIZE, data_len));
                }

                self.decompressor
                    .decompress(&data[..], &mut self.decompress_buf, data_len)?;

                let mut cursor: &[u8] = &self.decompress_buf;
                let packet_id = VarInt::decode(&mut cursor)?;
                let payload = bytes::Bytes::copy_from_slice(cursor);

                let conforming = self
                    .compression_threshold
                    .is_some_and(|t| t >= 0 && data_len >= t as usize);
                Ok(Some(if conforming {
                    PacketFrame::with_raw(packet_id.0, payload, wire, self.compression_threshold)
                } else {
                    PacketFrame::new(packet_id.0, payload)
                }))
            }
        }
    }

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

        let mid = raw.len() / 2;
        decoder.queue_bytes(&raw[..mid]);
        assert!(decoder.try_next_frame().unwrap().is_none());

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
        decoder.queue_bytes(&[0x80]);
        assert!(decoder.try_next_frame().unwrap().is_none());
    }

    #[test]
    fn test_decode_packet_too_large() {
        let big_len = VarInt((MAX_PACKET_SIZE + 1) as i32);
        let mut buf = Vec::new();
        big_len.encode(&mut buf).unwrap();

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
        let huge_data_len = VarInt((MAX_PACKET_DATA_SIZE + 1) as i32);
        let mut inner = Vec::new();
        huge_data_len.encode(&mut inner).unwrap();
        inner.extend_from_slice(&[0x00; 10]);

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
        let data_len = VarInt(100);
        let mut inner = Vec::new();
        data_len.encode(&mut inner).unwrap();
        inner.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

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

    fn wire_frame(id: i32, payload: &[u8], threshold: Option<i32>) -> Vec<u8> {
        let mut encoder = PacketEncoder::new();
        if let Some(t) = threshold {
            encoder.set_compression(t);
        }
        encoder.append_raw(id, payload).unwrap();
        encoder.take().to_vec()
    }

    fn forward(bytes: &[u8], decode_t: Option<i32>, encode_t: Option<i32>) -> Vec<u8> {
        let mut decoder = PacketDecoder::new();
        if let Some(t) = decode_t {
            decoder.set_compression(t);
        }
        decoder.queue_bytes(bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();

        let mut encoder = PacketEncoder::new();
        if let Some(t) = encode_t {
            encoder.set_compression(t);
        }
        encoder.append_frame(&frame).unwrap();
        encoder.take().to_vec()
    }

    #[test]
    fn test_raw_forward_is_byte_identical_at_same_threshold() {
        let bytes = wire_frame(0x10, b"some payload", None);
        assert_eq!(forward(&bytes, None, None), bytes);

        let big = vec![0x42; 512];
        let bytes = wire_frame(0x10, &big, Some(64));
        assert_eq!(forward(&bytes, Some(64), Some(64)), bytes);

        let bytes = wire_frame(0x10, b"tiny", Some(64));
        assert_eq!(forward(&bytes, Some(64), Some(64)), bytes);
    }

    #[test]
    fn test_raw_forward_reencodes_on_threshold_mismatch() {
        let big = vec![0x42; 512];

        let bytes = wire_frame(0x03, &big, Some(256));
        let out = forward(&bytes, Some(256), Some(64));
        let mut check = PacketDecoder::new();
        check.set_compression(64);
        check.queue_bytes(&out);
        let frame = check.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x03);
        assert_eq!(&frame.payload[..], &big[..]);

        let bytes = wire_frame(0x03, &big, None);
        let out = forward(&bytes, None, Some(64));
        let mut check = PacketDecoder::new();
        check.set_compression(64);
        check.queue_bytes(&out);
        let frame = check.try_next_frame().unwrap().unwrap();
        assert_eq!(&frame.payload[..], &big[..]);

        let bytes = wire_frame(0x03, &big, Some(64));
        let out = forward(&bytes, Some(64), None);
        let mut check = PacketDecoder::new();
        check.queue_bytes(&out);
        let frame = check.try_next_frame().unwrap().unwrap();
        assert_eq!(&frame.payload[..], &big[..]);
    }

    #[test]
    fn test_nonconforming_compressed_frame_is_normalized() {
        let payload = vec![0x42; 99];
        let mut enc = PacketEncoder::new();
        enc.set_compression(64);
        enc.append_raw(0x07, &payload).unwrap();
        let wire = enc.take().to_vec();

        let out = forward(&wire, Some(256), Some(256));
        assert_ne!(out, wire, "nonconforming framing must be re-encoded");

        let mut check = PacketDecoder::new();
        check.set_compression(256);
        check.queue_bytes(&out);
        let frame = check.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x07);
        assert_eq!(&frame.payload[..], &payload[..]);
    }

    #[test]
    fn test_rebuilt_frame_never_emits_stale_raw() {
        let bytes = wire_frame(0x10, &[0x42; 512], Some(64));
        let mut decoder = PacketDecoder::new();
        decoder.set_compression(64);
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();

        let modified = PacketFrame::new(frame.id, bytes::Bytes::from(vec![0x99; 512]));
        assert!(modified.raw_wire(Some(64)).is_none());

        let mut encoder = PacketEncoder::new();
        encoder.set_compression(64);
        encoder.append_frame(&modified).unwrap();
        let out = encoder.take();

        let mut check = PacketDecoder::new();
        check.set_compression(64);
        check.queue_bytes(&out);
        let decoded = check.try_next_frame().unwrap().unwrap();
        assert_eq!(&decoded.payload[..], &[0x99; 512][..]);
    }

    #[test]
    fn test_raw_wire_is_gated_on_threshold_and_survives_clone() {
        let big = vec![0x42; 512];
        let bytes = wire_frame(0x10, &big, Some(64));
        let mut decoder = PacketDecoder::new();
        decoder.set_compression(64);
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();

        assert!(frame.raw_wire(Some(64)).is_some());
        assert!(frame.raw_wire(Some(256)).is_none(), "threshold gate");
        assert!(frame.raw_wire(None).is_none(), "threshold gate");
        assert!(frame.clone().raw_wire(Some(64)).is_some());

        assert!(
            PacketFrame::new(frame.id, frame.payload.clone())
                .raw_wire(Some(64))
                .is_none(),
            "a rebuilt frame carries no wire bytes and must be re-encoded"
        );
    }

    #[test]
    fn test_full_pipeline_encode_decode_with_compression() {
        let mut encoder = PacketEncoder::new();
        let mut decoder = PacketDecoder::new();

        encoder.append_raw(0x00, b"hello").unwrap();
        let bytes = encoder.take();
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x00);
        assert_eq!(&frame.payload[..], b"hello");

        encoder.set_compression(256);
        decoder.set_compression(256);

        encoder.append_raw(0x01, b"small").unwrap();
        let bytes = encoder.take();
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x01);
        assert_eq!(&frame.payload[..], b"small");

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

    #[test]
    fn test_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PacketDecoder>();
        assert_send_sync::<PacketEncoder>();
        assert_send_sync::<crate::crypto::EncryptCipher>();
        assert_send_sync::<crate::crypto::DecryptCipher>();
    }
}
