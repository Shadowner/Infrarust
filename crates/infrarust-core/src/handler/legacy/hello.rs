use std::time::Duration;

use infrarust_protocol::ProtocolError;
use infrarust_protocol::ProtocolVersion;
use infrarust_protocol::legacy::{
    MAX_HOSTNAME_CHARS, MAX_PING_CHANNEL_CHARS, MAX_PING_DATA_BYTES, MAX_USERNAME_CHARS,
    parse_legacy_handshake, parse_legacy_ping,
};

use super::LegacyReader;
use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{ConnectionIntent, HandshakeData, LegacyHello};
use crate::util::normalize_handshake;

const PING_WAIT: Duration = Duration::from_millis(100);

pub(crate) async fn read_ping_hello(ctx: &mut ConnectionContext) -> Result<(), CoreError> {
    let raw = read_ping_bytes(ctx).await?;
    let request = parse_legacy_ping(&raw[1..])?;
    tracing::debug!(
        variant = ?request.variant,
        hostname = ?request.hostname,
        "legacy ping parsed"
    );
    let host = request.hostname.clone().unwrap_or_default();
    let port = request.port.and_then(|port| u16::try_from(port).ok());
    let protocol = request.protocol_version.map_or(0, i32::from);
    insert_hello(
        ctx,
        ConnectionIntent::Status,
        host,
        port,
        protocol,
        LegacyHello::Ping { request, raw },
    );
    Ok(())
}

pub(crate) async fn read_login_hello(ctx: &mut ConnectionContext) -> Result<(), CoreError> {
    let raw = read_login_bytes(ctx).await?;
    let request = parse_legacy_handshake(&raw[1..])?;
    tracing::debug!(
        protocol = request.protocol_version,
        username = %request.username,
        hostname = %request.hostname,
        port = request.port,
        "legacy login handshake"
    );
    let host = request.hostname.clone();
    let port = u16::try_from(request.port).ok();
    let protocol = i32::from(request.protocol_version);
    insert_hello(
        ctx,
        ConnectionIntent::Login,
        host,
        port,
        protocol,
        LegacyHello::Login { request, raw },
    );
    Ok(())
}

fn insert_hello(
    ctx: &mut ConnectionContext,
    intent: ConnectionIntent,
    host: String,
    port: Option<u16>,
    protocol: i32,
    hello: LegacyHello,
) {
    ctx.buffered_data.clear();
    ctx.extensions.insert(HandshakeData {
        domain: normalize_handshake(&host).to_lowercase(),
        raw_host: host,
        port: port.unwrap_or(0),
        protocol_version: ProtocolVersion(protocol),
        intent,
        raw_packets: Vec::new(),
    });
    ctx.extensions.insert(hello);
}

async fn read_ping_bytes(ctx: &mut ConnectionContext) -> Result<Vec<u8>, CoreError> {
    let mut reader = LegacyReader::new(ctx);
    let mut data = Vec::with_capacity(128);
    data.push(0xFE);

    if let Ok(Ok(next)) = tokio::time::timeout(PING_WAIT, reader.read_u8()).await {
        data.push(next);

        if next == 0x01 {
            match tokio::time::timeout(PING_WAIT, read_remaining_v1_6_data(&mut reader)).await {
                Ok(Ok(more)) => data.extend_from_slice(&more),
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::InvalidData => return Err(e.into()),
                Ok(Err(_)) | Err(_) => {}
            }
        }
    }

    Ok(data)
}

async fn read_remaining_v1_6_data(reader: &mut LegacyReader<'_>) -> std::io::Result<Vec<u8>> {
    let mut data = Vec::new();

    let marker = reader.read_u8().await?;
    data.push(marker);
    if marker != 0xFA {
        return Ok(data);
    }

    let mut channel_len = [0u8; 2];
    reader.read_exact(&mut channel_len).await?;
    data.extend_from_slice(&channel_len);
    let channel_chars = usize::from(u16::from_be_bytes(channel_len));
    refuse_over_limit(channel_chars, MAX_PING_CHANNEL_CHARS)?;
    data.extend_from_slice(&reader.read_vec(channel_chars * 2).await?);

    let mut payload_len = [0u8; 2];
    reader.read_exact(&mut payload_len).await?;
    data.extend_from_slice(&payload_len);
    let payload = usize::from(u16::from_be_bytes(payload_len));
    refuse_over_limit(payload, MAX_PING_DATA_BYTES)?;
    data.extend_from_slice(&reader.read_vec(payload).await?);

    Ok(data)
}

fn refuse_over_limit(declared: usize, limit: usize) -> std::io::Result<()> {
    if declared > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            ProtocolError::too_large(limit, declared),
        ));
    }
    Ok(())
}

async fn read_login_bytes(ctx: &mut ConnectionContext) -> Result<Vec<u8>, CoreError> {
    let mut reader = LegacyReader::new(ctx);
    let mut data = Vec::with_capacity(256);
    data.push(0x02);

    let format = reader.read_u8().await?;
    data.push(format);

    if format == 0x00 {
        let chars = reader.read_u8().await?;
        data.push(chars);
        data.extend_from_slice(&reader.read_vec(usize::from(chars) * 2).await?);
    } else {
        read_string_into(&mut reader, &mut data, MAX_USERNAME_CHARS).await?;
        read_string_into(&mut reader, &mut data, MAX_HOSTNAME_CHARS).await?;
        data.extend_from_slice(&reader.read_vec(4).await?);
    }

    data.extend_from_slice(&reader.unread());
    Ok(data)
}

async fn read_string_into(
    reader: &mut LegacyReader<'_>,
    data: &mut Vec<u8>,
    max_chars: usize,
) -> Result<(), CoreError> {
    let mut len_bytes = [0u8; 2];
    reader.read_exact(&mut len_bytes).await?;
    let chars = usize::from(u16::from_be_bytes(len_bytes));
    refuse_over_limit(chars, max_chars)?;
    data.extend_from_slice(&len_bytes);
    data.extend_from_slice(&reader.read_vec(chars * 2).await?);
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::net::{IpAddr, Ipv4Addr};

    use infrarust_protocol::legacy::{LegacyPingRequest, LegacyPingVariant};
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    fn utf16be(s: &str) -> Vec<u8> {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut out = u16::try_from(units.len()).unwrap().to_be_bytes().to_vec();
        for unit in units {
            out.extend_from_slice(&unit.to_be_bytes());
        }
        out
    }

    fn v1_6_ping(hostname: &str, port: i32) -> Vec<u8> {
        let mut data = vec![78];
        data.extend_from_slice(&utf16be(hostname));
        data.extend_from_slice(&port.to_be_bytes());
        let mut ping = vec![0xFE, 0x01, 0xFA];
        ping.extend_from_slice(&utf16be("MC|PingHost"));
        ping.extend_from_slice(&u16::try_from(data.len()).unwrap().to_be_bytes());
        ping.extend_from_slice(&data);
        ping
    }

    async fn ctx_with(sent: &[u8], read_ahead: usize) -> (ConnectionContext, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(local).await.unwrap();
        let (stream, peer) = listener.accept().await.unwrap();
        let mut ctx =
            ConnectionContext::new_for_test(stream, peer, IpAddr::V4(Ipv4Addr::LOCALHOST), local);
        ctx.buffered_data.extend_from_slice(&sent[..read_ahead]);
        client.write_all(&sent[read_ahead..]).await.unwrap();
        (ctx, client)
    }

    async fn read_ping(sent: &[u8], read_ahead: usize) -> LegacyPingRequest {
        let (mut ctx, client) = ctx_with(sent, read_ahead).await;
        let raw = read_ping_bytes(&mut ctx).await.unwrap();
        assert_eq!(raw, sent, "read ahead {read_ahead}");
        drop(client);
        parse_legacy_ping(&raw[1..]).unwrap()
    }

    #[tokio::test]
    async fn a_v1_6_ping_is_read_whole_however_much_was_read_ahead() {
        let ping = v1_6_ping("lobby.test", 25565);
        for read_ahead in [1, 2, 3, 4, 7, 20, ping.len() - 1, ping.len()] {
            let request = read_ping(&ping, read_ahead).await;
            assert_eq!(request.variant, LegacyPingVariant::V1_6, "{read_ahead}");
            assert_eq!(request.hostname.as_deref(), Some("lobby.test"));
            assert_eq!(request.port, Some(25565));
            assert_eq!(request.protocol_version, Some(78));
        }
    }

    #[tokio::test]
    async fn a_v1_4_and_a_beta_ping_are_read_from_what_was_read_ahead() {
        for read_ahead in [1, 2] {
            let request = read_ping(&[0xFE, 0x01], read_ahead).await;
            assert_eq!(request.variant, LegacyPingVariant::V1_4, "{read_ahead}");
        }
        let request = read_ping(&[0xFE], 1).await;
        assert_eq!(request.variant, LegacyPingVariant::Beta);
    }

    #[tokio::test]
    async fn oversized_login_fields_are_refused_without_waiting_for_their_bytes() {
        let name_header = [&[0x02, 78][..], &17u16.to_be_bytes()[..]].concat();
        let mut host_header = vec![0x02, 78];
        host_header.extend_from_slice(&utf16be("Notch"));
        host_header.extend_from_slice(&256u16.to_be_bytes());
        for hello in [name_header, host_header] {
            let (mut ctx, _client) = ctx_with(&hello, 1).await;
            let refused = tokio::time::timeout(Duration::from_secs(1), read_login_hello(&mut ctx))
                .await
                .expect("the proxy must not wait for the declared bytes");
            assert!(refused.is_err());
        }
    }

    #[tokio::test]
    async fn an_oversized_pre_1_3_login_is_refused() {
        let mut hello = vec![0x02, 0x00, 17];
        for unit in "a".repeat(17).encode_utf16() {
            hello.extend_from_slice(&unit.to_be_bytes());
        }
        let (mut ctx, _client) = ctx_with(&hello, 1).await;
        assert!(read_login_hello(&mut ctx).await.is_err());
    }

    #[tokio::test]
    async fn oversized_v1_6_pings_are_refused_without_waiting_for_their_bytes() {
        let channel_header = [&[0xFE, 0x01, 0xFA][..], &21u16.to_be_bytes()[..]].concat();
        let mut data_header = vec![0xFE, 0x01, 0xFA];
        data_header.extend_from_slice(&utf16be("MC|PingHost"));
        data_header.extend_from_slice(&518u16.to_be_bytes());
        for hello in [channel_header, data_header] {
            let (mut ctx, _client) = ctx_with(&hello, 1).await;
            assert!(read_ping_hello(&mut ctx).await.is_err());
        }

        let (mut ctx, _client) = ctx_with(&v1_6_ping(&"b".repeat(256), 25565), 1).await;
        assert!(read_ping_hello(&mut ctx).await.is_err());
    }

    #[tokio::test]
    async fn a_ping_hello_is_an_ordinary_status_handshake() {
        let (mut ctx, _client) = ctx_with(&v1_6_ping("LOBBY.test", 25570), 1).await;
        read_ping_hello(&mut ctx).await.unwrap();

        let handshake = ctx.extensions.get::<HandshakeData>().unwrap();
        assert_eq!(handshake.intent, ConnectionIntent::Status);
        assert_eq!(handshake.domain, "lobby.test");
        assert_eq!(handshake.raw_host, "LOBBY.test");
        assert_eq!(handshake.port, 25570);
        assert_eq!(handshake.protocol_version, ProtocolVersion(78));
        assert!(ctx.buffered_data.is_empty());
        assert!(!ctx.extensions.get::<LegacyHello>().unwrap().is_hostless());
    }

    #[tokio::test]
    async fn a_ping_without_a_host_is_marked_hostless() {
        let (mut ctx, _client) = ctx_with(&[0xFE], 1).await;
        read_ping_hello(&mut ctx).await.unwrap();

        assert!(ctx.extensions.get::<LegacyHello>().unwrap().is_hostless());
    }
}
