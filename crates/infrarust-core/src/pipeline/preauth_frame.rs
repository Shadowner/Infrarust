use bytes::BytesMut;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

use infrarust_protocol::ProtocolError;
use infrarust_protocol::codec::VarInt;
use infrarust_protocol::io::{PacketDecoder, PacketFrame};

use crate::error::CoreError;

pub const MAX_HANDSHAKE_FRAME: usize = 2 * 1024;
pub const MAX_LOGIN_START_FRAME: usize = 5 * 1024;
pub const MAX_STATUS_FRAME: usize = 64;
pub const MAX_ENCRYPTION_RESPONSE_FRAME: usize = 2 * 1024;

/// Reads the next frame, refusing it as soon as its declared length exceeds
/// `limit`. Bytes read from the stream are also appended to `raw`.
pub async fn read_preauth_frame(
    stream: &mut TcpStream,
    decoder: &mut PacketDecoder,
    raw: &mut BytesMut,
    limit: usize,
) -> Result<PacketFrame, CoreError> {
    loop {
        refuse_oversized(decoder.read_buf_mut(), limit)?;
        if let Some(frame) = decoder.try_next_frame()? {
            return Ok(frame);
        }
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Err(CoreError::ConnectionClosed);
        }
        decoder.queue_bytes(&buf[..n]);
        raw.extend_from_slice(&buf[..n]);
    }
}

pub(crate) fn refuse_oversized(buffered: &[u8], limit: usize) -> Result<(), ProtocolError> {
    if let Ok((VarInt(declared), _)) = VarInt::decode_partial(buffered)
        && let Ok(declared) = usize::try_from(declared)
        && declared > limit
    {
        return Err(ProtocolError::too_large(limit, declared));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use infrarust_protocol::Packet;
    use infrarust_protocol::packets::login::{EncryptionProof, SEncryptionResponse};
    use infrarust_protocol::version::ProtocolVersion;

    fn declared(length: i32) -> Vec<u8> {
        let mut out = Vec::new();
        VarInt(length).encode(&mut out).unwrap();
        out
    }

    #[test]
    fn the_largest_signed_encryption_response_fits() {
        let response = SEncryptionResponse {
            shared_secret: vec![0; 128],
            proof: EncryptionProof::Signature {
                salt: 0,
                signature: vec![0; 1024],
            },
        };
        let mut frame = vec![0x01];
        response.encode(&mut frame, ProtocolVersion::V1_19).unwrap();
        assert!(frame.len() <= MAX_ENCRYPTION_RESPONSE_FRAME);
    }

    #[test]
    fn only_a_declared_length_over_the_limit_is_refused() {
        assert!(refuse_oversized(&declared(2048), 2048).is_ok());
        assert!(matches!(
            refuse_oversized(&declared(2049), 2048),
            Err(ProtocolError::TooLarge {
                max: 2048,
                actual: 2049
            })
        ));
    }

    #[test]
    fn what_the_decoder_reports_itself_is_left_to_it() {
        assert!(refuse_oversized(&[], 2048).is_ok());
        assert!(refuse_oversized(&[0x80], 2048).is_ok());
        assert!(refuse_oversized(&declared(0), 2048).is_ok());
        assert!(refuse_oversized(&declared(-1), 2048).is_ok());
    }

    #[test]
    fn bytes_after_the_length_do_not_count() {
        let mut buffered = declared(16);
        buffered.extend_from_slice(&[0u8; 8192]);
        assert!(refuse_oversized(&buffered, 2048).is_ok());
    }
}
