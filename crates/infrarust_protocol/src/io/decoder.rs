use bytes::{Buf, Bytes, BytesMut};

use crate::codec::VarInt;
use crate::codec::varint::VarIntDecodeStatus;
use crate::error::{ProtocolError, ProtocolResult};
use crate::io::BUFFER_RETENTION_LIMIT;
use crate::io::compression::{self, ZlibDecompressor};
use crate::io::frame::{PacketFrame, WireFormat};
use crate::{MAX_PACKET_DATA_SIZE, MAX_PACKET_SIZE};

pub struct PacketDecoder {
    buf: BytesMut,
    held_oversized: bool,
    compression_threshold: Option<i32>,
    decompressor: Option<Box<dyn ZlibDecompressor + Send + Sync>>,
}

const fn is_canonical(value: VarInt, encoded: &[u8]) -> bool {
    if encoded.len() != value.written_size() {
        return false;
    }
    match encoded.last() {
        Some(&last) => encoded.len() < VarInt::MAX_SIZE || last & 0x70 == 0,
        None => false,
    }
}

#[inline]
fn read_varint(bytes: &[u8]) -> ProtocolResult<(VarInt, usize)> {
    let mut cursor = bytes;
    let value = VarInt::decode(&mut cursor)?;
    Ok((value, bytes.len() - cursor.len()))
}

impl PacketDecoder {
    pub fn new() -> Self {
        Self {
            buf: BytesMut::new(),
            held_oversized: false,
            compression_threshold: None,
            decompressor: None,
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

        self.held_oversized |= self.buf.len() > BUFFER_RETENTION_LIMIT;
        let wire = self.buf.split_to(varint_size + packet_len).freeze();
        if self.held_oversized && self.buf.is_empty() {
            self.buf = BytesMut::new();
            self.held_oversized = false;
        }
        let canonical_framing = is_canonical(packet_len_varint, &wire[..varint_size]);

        if self.compression_threshold.is_none() {
            let (packet_id, id_size) = read_varint(&wire[varint_size..])?;
            let conforming = canonical_framing
                && is_canonical(packet_id, &wire[varint_size..varint_size + id_size]);
            let payload = wire.slice(varint_size + id_size..);
            return Ok(Some(if conforming {
                PacketFrame::with_raw(packet_id.0, payload, wire, WireFormat::Plain)
            } else {
                PacketFrame::new(packet_id.0, payload)
            }));
        }

        let (data_len, data_len_size) = read_varint(&wire[varint_size..])?;
        let body_start = varint_size + data_len_size;
        let canonical_framing =
            canonical_framing && is_canonical(data_len, &wire[varint_size..body_start]);
        let body = &wire[body_start..];

        let (packet_id, canonical_id, payload, format) = if data_len.0 == 0 {
            let (packet_id, id_size) = read_varint(body)?;
            let format = WireFormat::Compressed {
                deflated: false,
                uncompressed_len: body.len(),
            };
            let canonical_id = is_canonical(packet_id, &body[..id_size]);
            let payload = wire.slice(body_start + id_size..);
            (packet_id, canonical_id, payload, format)
        } else {
            let data_len = data_len.0 as usize;
            if data_len > MAX_PACKET_DATA_SIZE {
                return Err(ProtocolError::too_large(MAX_PACKET_DATA_SIZE, data_len));
            }

            let mut inflated = Vec::with_capacity(data_len);
            self.decompressor
                .get_or_insert_with(compression::new_decompressor)
                .decompress(body, &mut inflated, data_len)?;

            let (packet_id, id_size) = read_varint(&inflated)?;
            let format = WireFormat::Compressed {
                deflated: true,
                uncompressed_len: data_len,
            };
            let canonical_id = is_canonical(packet_id, &inflated[..id_size]);
            let mut payload = Bytes::from(inflated);
            payload.advance(id_size);
            (packet_id, canonical_id, payload, format)
        };

        let conforming =
            canonical_framing && canonical_id && format.conforms_to(self.compression_threshold);
        Ok(Some(if conforming {
            PacketFrame::with_raw(packet_id.0, payload, wire, format)
        } else {
            PacketFrame::new(packet_id.0, payload)
        }))
    }

    pub const fn set_compression(&mut self, threshold: i32) {
        self.compression_threshold = if threshold < 0 { None } else { Some(threshold) };
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

    fn decode_one(bytes: &[u8], threshold: Option<i32>) -> PacketFrame {
        let mut decoder = PacketDecoder::new();
        if let Some(t) = threshold {
            decoder.set_compression(t);
        }
        decoder.queue_bytes(bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert!(decoder.try_next_frame().unwrap().is_none());
        frame
    }

    fn assert_reencoded(
        bytes: &[u8],
        payload: &[u8],
        decode_t: Option<i32>,
        encode_t: Option<i32>,
    ) {
        assert!(
            decode_one(bytes, decode_t).raw_wire(encode_t).is_none(),
            "{decode_t:?} -> {encode_t:?} must not reuse the wire bytes"
        );
        let out = forward(bytes, decode_t, encode_t);
        assert_ne!(out, bytes, "{decode_t:?} -> {encode_t:?}");

        let frame = decode_one(&out, encode_t);
        assert_eq!(frame.id, 0x03);
        assert_eq!(&frame.payload[..], payload);
        assert!(
            frame.raw_wire(encode_t).is_some(),
            "{decode_t:?} -> {encode_t:?} must produce conforming framing"
        );
    }

    #[test]
    fn test_raw_forward_is_byte_identical_when_valid_for_destination_threshold() {
        let big = vec![0x42; 512];
        let deflated = wire_frame(0x03, &big, Some(256));
        for destination in [0, 64, 256, 512, 513] {
            assert_eq!(
                forward(&deflated, Some(256), Some(destination)),
                deflated,
                "deflated body of 513 bytes at threshold {destination}"
            );
        }

        let stored = wire_frame(0x03, b"tiny", Some(64));
        for destination in [6, 64, 256, i32::MAX] {
            assert_eq!(
                forward(&stored, Some(64), Some(destination)),
                stored,
                "stored body of 5 bytes at threshold {destination}"
            );
        }
    }

    #[test]
    fn test_raw_forward_reencodes_when_invalid_for_destination_threshold() {
        let big = vec![0x42; 512];

        let deflated = wire_frame(0x03, &big, Some(256));
        assert_reencoded(&deflated, &big, Some(256), Some(514));
        assert_reencoded(&deflated, &big, Some(256), Some(4096));
        assert_reencoded(&deflated, &big, Some(256), None);

        let stored = wire_frame(0x03, b"tiny", Some(64));
        assert_reencoded(&stored, b"tiny", Some(64), Some(5));
        assert_reencoded(&stored, b"tiny", Some(64), Some(0));
        assert_reencoded(&stored, b"tiny", Some(64), None);

        let plain = wire_frame(0x03, &big, None);
        assert_reencoded(&plain, &big, None, Some(64));
        assert_reencoded(&plain, &big, None, Some(4096));

        let plain = wire_frame(0x03, b"tiny", None);
        assert_reencoded(&plain, b"tiny", None, Some(64));
        assert_reencoded(&plain, b"tiny", None, Some(0));
    }

    #[test]
    fn test_framing_nonconforming_at_capture_is_never_reused() {
        let payload = vec![0x42; 99];

        let deflated_below_threshold = wire_frame(0x07, &payload, Some(64));
        let frame = decode_one(&deflated_below_threshold, Some(256));
        assert_eq!(&frame.payload[..], &payload[..]);
        for destination in [None, Some(0), Some(64), Some(100), Some(256)] {
            assert!(frame.raw_wire(destination).is_none(), "{destination:?}");
        }

        let stored_above_threshold = wire_frame(0x07, &payload, Some(256));
        let frame = decode_one(&stored_above_threshold, Some(64));
        assert_eq!(&frame.payload[..], &payload[..]);
        for destination in [None, Some(0), Some(64), Some(256), Some(4096)] {
            assert!(frame.raw_wire(destination).is_none(), "{destination:?}");
        }

        let mut overlong = overlong_varint(0, VarInt::MAX_SIZE);
        overlong.push(0x07);
        overlong.extend_from_slice(b"tiny");
        let mut wire = vec![u8::try_from(overlong.len()).unwrap()];
        wire.extend_from_slice(&overlong);
        let frame = decode_one(&wire, Some(64));
        for destination in [None, Some(64), Some(256)] {
            assert!(frame.raw_wire(destination).is_none(), "{destination:?}");
        }
    }

    fn compressed_frame(declared_len: i32, body: &[u8]) -> Vec<u8> {
        let mut inner = Vec::new();
        VarInt(declared_len).encode(&mut inner).unwrap();
        inner.extend_from_slice(body);
        let mut wire = Vec::new();
        VarInt(inner.len() as i32).encode(&mut wire).unwrap();
        wire.extend_from_slice(&inner);
        wire
    }

    #[test]
    fn test_declared_size_must_match_inflated_size() {
        let big: Vec<u8> = (0..512).map(|i: u32| (i % 251) as u8).collect();
        let wire = wire_frame(0x03, &big, Some(64));
        let (_, len_size) = VarInt::decode_partial(&wire).unwrap();
        let (data_len, data_len_size) = VarInt::decode_partial(&wire[len_size..]).unwrap();
        assert_eq!(data_len.0, 513);
        let body = &wire[len_size + data_len_size..];

        assert_eq!(compressed_frame(513, body), wire);
        assert_eq!(&decode_one(&wire, Some(64)).payload[..], &big[..]);

        let truncated = &body[..body.len() - 8];
        for tampered in [
            compressed_frame(512, body),
            compressed_frame(514, body),
            compressed_frame(513, truncated),
        ] {
            let mut decoder = PacketDecoder::new();
            decoder.set_compression(64);
            decoder.queue_bytes(&tampered);
            assert!(decoder.try_next_frame().is_err());
        }
    }

    #[test]
    fn test_oversized_read_buffer_is_released_once_drained() {
        let big = vec![0x42; 4 * BUFFER_RETENTION_LIMIT];
        let mut bytes = wire_frame(0x03, &big, None);
        bytes.extend_from_slice(&wire_frame(0x04, b"tiny", None));

        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&bytes);
        let first = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(&first.payload[..], &big[..]);
        let second = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(&second.payload[..], b"tiny");
        assert_eq!(decoder.buf.capacity(), 0);

        decoder.queue_bytes(&wire_frame(0x04, b"tiny", None));
        let third = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(&third.payload[..], b"tiny");
    }

    #[test]
    fn test_read_buffer_that_held_a_backlog_is_released_once_drained() {
        let frame = wire_frame(0x04, &[0x42; 1024], None);
        let count = 2 * BUFFER_RETENTION_LIMIT / frame.len();

        let mut decoder = PacketDecoder::new();
        for _ in 0..count {
            decoder.queue_bytes(&frame);
        }
        for _ in 0..count - 1 {
            decoder.try_next_frame().unwrap().unwrap();
        }
        assert!(decoder.buf.capacity() > 0);
        decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(decoder.buf.capacity(), 0);

        decoder.queue_bytes(&frame);
        decoder.try_next_frame().unwrap().unwrap();
        assert!(!decoder.held_oversized);
    }

    #[test]
    fn test_decompressor_is_created_on_first_inflate() {
        let mut decoder = PacketDecoder::new();
        decoder.set_compression(64);
        decoder.queue_bytes(&wire_frame(0x03, b"tiny", Some(64)));
        decoder.try_next_frame().unwrap().unwrap();
        assert!(decoder.decompressor.is_none());

        decoder.queue_bytes(&wire_frame(0x03, &[0x42; 512], Some(64)));
        decoder.try_next_frame().unwrap().unwrap();
        assert!(decoder.decompressor.is_some());
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

    fn overlong_varint(value: i32, encoded_size: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(encoded_size);
        let mut remaining = value as u32;
        for _ in 0..encoded_size - 1 {
            out.push(((remaining & 0x7F) as u8) | 0x80);
            remaining >>= 7;
        }
        out.push((remaining & 0x7F) as u8);
        out
    }

    #[test]
    fn test_nonconforming_uncompressed_frame_is_normalized() {
        let payload = vec![0x42; 200];
        let mut enc = PacketEncoder::new();
        enc.set_compression(256);
        enc.append_raw(0x07, &payload).unwrap();
        let wire = enc.take().to_vec();

        let out = forward(&wire, Some(64), Some(64));
        assert_ne!(out, wire, "nonconforming framing must be re-encoded");

        let mut check = PacketDecoder::new();
        check.set_compression(64);
        check.queue_bytes(&out);
        let frame = check.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x07);
        assert_eq!(&frame.payload[..], &payload[..]);
    }

    #[test]
    fn test_overlong_length_varint_is_normalized() {
        let mut wire = overlong_varint(5, VarInt::MAX_SIZE);
        wire.push(0x00);
        wire.extend_from_slice(b"abcd");

        let out = forward(&wire, None, None);
        assert_ne!(out, wire, "overlong length VarInt must be re-encoded");
        assert_eq!(out, encode_frame(0x00, b"abcd"));
    }

    #[test]
    fn test_overlong_packet_id_varint_is_normalized() {
        let packet_id = overlong_varint(0, 2);
        let mut wire = vec![u8::try_from(packet_id.len() + 4).unwrap()];
        wire.extend_from_slice(&packet_id);
        wire.extend_from_slice(b"abcd");

        let out = forward(&wire, None, None);
        assert_ne!(out, wire, "overlong packet id VarInt must be re-encoded");
        assert_eq!(out, encode_frame(0x00, b"abcd"));
    }

    #[test]
    fn test_overlong_data_len_varint_is_normalized() {
        let mut inner = overlong_varint(0, VarInt::MAX_SIZE);
        inner.push(0x07);
        inner.extend_from_slice(b"tiny");
        let mut wire = vec![u8::try_from(inner.len()).unwrap()];
        wire.extend_from_slice(&inner);

        let out = forward(&wire, Some(256), Some(256));
        assert_ne!(out, wire, "overlong data length VarInt must be re-encoded");

        let mut check = PacketDecoder::new();
        check.set_compression(256);
        check.queue_bytes(&out);
        let frame = check.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x07);
        assert_eq!(&frame.payload[..], b"tiny");
    }

    #[test]
    fn test_noncanonical_five_byte_varint_is_normalized() {
        let payload = b"abcd";
        let canonical = wire_frame(i32::MAX, payload, None);
        assert_eq!(forward(&canonical, None, None), canonical);

        let last = canonical.len() - payload.len() - 1;
        assert_eq!(canonical[last], 0x07);
        let mut tampered = canonical.clone();
        tampered[last] = 0x77;

        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&tampered);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, i32::MAX);
        assert_eq!(&frame.payload[..], payload);
        assert!(
            frame.raw_wire(None).is_none(),
            "5-byte VarInt with bits past i32 must not be forwarded verbatim"
        );

        let out = forward(&tampered, None, None);
        assert_ne!(out, tampered);
        assert_eq!(out, canonical);
    }

    #[test]
    fn test_negative_compression_threshold_is_treated_as_disabled() {
        let mut decoder = PacketDecoder::new();
        decoder.set_compression(-1);
        assert_eq!(decoder.compression_threshold(), None);

        let mut encoder = PacketEncoder::new();
        encoder.set_compression(-1);
        assert_eq!(encoder.compression_threshold(), None);

        encoder.append_raw(0x09, b"payload").unwrap();
        let bytes = encoder.take();
        assert_eq!(&bytes[..], &encode_frame(0x09, b"payload")[..]);

        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x09);
        assert_eq!(&frame.payload[..], b"payload");
    }

    #[test]
    fn test_zero_compression_threshold_compresses_everything() {
        let mut encoder = PacketEncoder::new();
        encoder.set_compression(0);
        assert_eq!(encoder.compression_threshold(), Some(0));
        encoder.append_raw(0x09, b"x").unwrap();
        let bytes = encoder.take();

        let mut decoder = PacketDecoder::new();
        decoder.set_compression(0);
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x09);
        assert_eq!(&frame.payload[..], b"x");
        assert!(frame.raw_wire(Some(0)).is_some());
    }

    #[test]
    fn test_rebuilt_frame_never_emits_stale_raw() {
        let bytes = wire_frame(0x10, &[0x42; 512], Some(64));
        let mut decoder = PacketDecoder::new();
        decoder.set_compression(64);
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();

        let modified = PacketFrame::new(frame.id, Bytes::from(vec![0x99; 512]));
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
    fn test_raw_wire_is_gated_on_destination_validity_and_survives_clone() {
        let big = vec![0x42; 512];
        let bytes = wire_frame(0x10, &big, Some(64));
        let mut decoder = PacketDecoder::new();
        decoder.set_compression(64);
        decoder.queue_bytes(&bytes);
        let frame = decoder.try_next_frame().unwrap().unwrap();

        assert!(frame.raw_wire(Some(64)).is_some());
        assert!(frame.raw_wire(Some(256)).is_some());
        assert!(frame.raw_wire(Some(1024)).is_none(), "threshold gate");
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
