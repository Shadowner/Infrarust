//! `HAProxy` proxy protocol v1/v2 decode and encode.
//!
//! Uses the `ppp` crate for parsing and building proxy protocol headers.
//! When proxy protocol is expected, it MUST be present — there is no
//! silent fallback.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::connection::ConnectionInfo;
use crate::error::TransportError;

/// Maximum proxy protocol header size (covers both v1 and v2).
const MAX_HEADER_SIZE: usize = 536;

/// Proxy protocol v2 signature (12 bytes).
const PP_V2_SIGNATURE: [u8; 12] = [
    0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A,
];

/// Proxy protocol v1 prefix.
const PP_V1_PREFIX: &[u8] = b"PROXY ";

/// Version of the proxy protocol header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyProtocolVersion {
    V1,
    V2,
}

/// Decoded proxy protocol information.
#[derive(Debug, Clone)]
pub struct ProxyProtocolInfo {
    /// Source (client) address.
    pub source_addr: SocketAddr,
    /// Destination address.
    pub dest_addr: SocketAddr,
    /// Protocol version.
    pub version: ProxyProtocolVersion,
}

/// Decodes a proxy protocol header from the stream.
///
/// Reads up to 536 bytes and attempts to parse as v2 (binary) then v1 (text).
/// Returns the parsed info (if any) and any leftover bytes that were read
/// past the end of the header (due to TCP coalescing).
///
/// # Errors
///
/// Returns an error if the header is missing, malformed, or uses an
/// unsupported address family. This is fatal -- the connection should
/// be closed.
pub async fn decode_proxy_protocol(
    stream: &mut TcpStream,
) -> Result<(Option<ProxyProtocolInfo>, Vec<u8>), TransportError> {
    timeout(Duration::from_secs(5), async {
        let mut buf = [0u8; MAX_HEADER_SIZE];
        let mut total_read = 0;

        loop {
            if total_read >= MAX_HEADER_SIZE {
                return Err(TransportError::InvalidProxyProtocol(
                    "header exceeds maximum size".to_string(),
                ));
            }

            let n = stream
                .read(&mut buf[total_read..])
                .await
                .map_err(|e| TransportError::ProxyProtocolDecode(e.to_string()))?;

            if n == 0 {
                return Err(TransportError::InvalidProxyProtocol(
                    "connection closed before proxy protocol header".to_string(),
                ));
            }

            total_read += n;

            // Try v2 first (binary)
            if total_read >= 16 && buf[..12] == PP_V2_SIGNATURE {
                match try_decode_v2(&buf[..total_read]) {
                    Ok((info, consumed)) => {
                        let leftover = buf[consumed..total_read].to_vec();
                        return Ok((Some(info), leftover));
                    }
                    Err(DecodeAttempt::Incomplete) => continue,
                    Err(DecodeAttempt::Failed(e)) => return Err(e),
                }
            }

            // Try v1 (text)
            if total_read >= 6 && buf[..6] == *PP_V1_PREFIX {
                match try_decode_v1(&buf[..total_read]) {
                    Ok((info, consumed)) => {
                        let leftover = buf[consumed..total_read].to_vec();
                        return Ok((info, leftover));
                    }
                    Err(DecodeAttempt::Incomplete) => continue,
                    Err(DecodeAttempt::Failed(e)) => return Err(e),
                }
            }

            // If we have enough bytes and neither signature matches, it's not PP
            if total_read >= 16 {
                return Err(TransportError::InvalidProxyProtocol(
                    "data does not start with a proxy protocol signature".to_string(),
                ));
            }
        }
    })
    .await
    .map_err(|_| {
        TransportError::ProxyProtocolDecode("proxy protocol header read timed out".to_string())
    })?
}

enum DecodeAttempt {
    Incomplete,
    Failed(TransportError),
}

fn try_decode_v2(buf: &[u8]) -> Result<(ProxyProtocolInfo, usize), DecodeAttempt> {
    let header = ppp::v2::Header::try_from(buf).map_err(|e| match e {
        ppp::v2::ParseError::Incomplete(_) | ppp::v2::ParseError::Partial(_, _) => {
            DecodeAttempt::Incomplete
        }
        other => DecodeAttempt::Failed(TransportError::ProxyProtocolDecode(format!("{other}"))),
    })?;

    let (source_addr, dest_addr) =
        extract_v2_addresses(&header.addresses).map_err(DecodeAttempt::Failed)?;

    let consumed = header.len();

    Ok((
        ProxyProtocolInfo {
            source_addr,
            dest_addr,
            version: ProxyProtocolVersion::V2,
        },
        consumed,
    ))
}

fn try_decode_v1(buf: &[u8]) -> Result<(Option<ProxyProtocolInfo>, usize), DecodeAttempt> {
    // v1 header ends with \r\n
    let crlf_pos = buf.windows(2).position(|w| w == b"\r\n");

    let end = match crlf_pos {
        Some(pos) => pos + 2,
        None => return Err(DecodeAttempt::Incomplete),
    };

    let header_str = std::str::from_utf8(&buf[..end])
        .map_err(|e| DecodeAttempt::Failed(TransportError::ProxyProtocolDecode(e.to_string())))?;

    let header = ppp::v1::Header::try_from(header_str).map_err(|e| {
        DecodeAttempt::Failed(TransportError::ProxyProtocolDecode(format!("{e:?}")))
    })?;

    // PROXY UNKNOWN is valid per HAProxy spec (used for health checks)
    if matches!(header.addresses, ppp::v1::Addresses::Unknown) {
        return Ok((None, end));
    }

    let (source_addr, dest_addr) =
        extract_v1_addresses(&header.addresses).map_err(DecodeAttempt::Failed)?;

    Ok((
        Some(ProxyProtocolInfo {
            source_addr,
            dest_addr,
            version: ProxyProtocolVersion::V1,
        }),
        end,
    ))
}

fn extract_v2_addresses(
    addresses: &ppp::v2::Addresses,
) -> Result<(SocketAddr, SocketAddr), TransportError> {
    match addresses {
        ppp::v2::Addresses::IPv4(ipv4) => {
            let src = SocketAddr::new(IpAddr::V4(ipv4.source_address), ipv4.source_port);
            let dst = SocketAddr::new(IpAddr::V4(ipv4.destination_address), ipv4.destination_port);
            Ok((src, dst))
        }
        ppp::v2::Addresses::IPv6(ipv6) => {
            let src = SocketAddr::new(IpAddr::V6(ipv6.source_address), ipv6.source_port);
            let dst = SocketAddr::new(IpAddr::V6(ipv6.destination_address), ipv6.destination_port);
            Ok((src, dst))
        }
        _ => Err(TransportError::InvalidProxyProtocol(
            "unsupported address family in v2 header".to_string(),
        )),
    }
}

fn extract_v1_addresses(
    addresses: &ppp::v1::Addresses,
) -> Result<(SocketAddr, SocketAddr), TransportError> {
    match addresses {
        ppp::v1::Addresses::Tcp4(tcp4) => {
            let src = SocketAddr::new(IpAddr::V4(tcp4.source_address), tcp4.source_port);
            let dst = SocketAddr::new(IpAddr::V4(tcp4.destination_address), tcp4.destination_port);
            Ok((src, dst))
        }
        ppp::v1::Addresses::Tcp6(tcp6) => {
            let src = SocketAddr::new(IpAddr::V6(tcp6.source_address), tcp6.source_port);
            let dst = SocketAddr::new(IpAddr::V6(tcp6.destination_address), tcp6.destination_port);
            Ok((src, dst))
        }
        ppp::v1::Addresses::Unknown => Err(TransportError::InvalidProxyProtocol(
            "unexpected unknown address family in v1 header".to_string(),
        )),
    }
}

/// Encodes and sends a proxy protocol v2 header to the backend stream.
///
/// Uses the client's real address (from proxy protocol) if available,
/// otherwise uses the peer address.
///
/// # Errors
///
/// Returns [`TransportError::ProxyProtocolDecode`] if header construction
/// fails, or [`TransportError::Forward`] if writing to the stream fails.
pub async fn encode_proxy_protocol_v2(
    stream: &mut TcpStream,
    client_info: &ConnectionInfo,
) -> Result<(), TransportError> {
    let source_ip = client_info
        .real_ip
        .unwrap_or_else(|| client_info.peer_addr.ip());
    let source_port = client_info
        .real_port
        .unwrap_or_else(|| client_info.peer_addr.port());
    let source_addr = SocketAddr::new(source_ip, source_port);
    let dest_addr = client_info.local_addr;

    let addresses: ppp::v2::Addresses = match (source_addr, dest_addr) {
        (SocketAddr::V4(src), SocketAddr::V4(dst)) => {
            ppp::v2::IPv4::new(src.ip().octets(), dst.ip().octets(), src.port(), dst.port()).into()
        }
        (SocketAddr::V6(src), SocketAddr::V6(dst)) => {
            ppp::v2::IPv6::new(src.ip().octets(), dst.ip().octets(), src.port(), dst.port()).into()
        }
        _ => {
            // Mixed v4/v6 — map v4 source to v6
            let src_v6 = match source_addr.ip() {
                IpAddr::V4(v4) => v4.to_ipv6_mapped(),
                IpAddr::V6(v6) => v6,
            };
            let dst_v6 = match dest_addr.ip() {
                IpAddr::V4(v4) => v4.to_ipv6_mapped(),
                IpAddr::V6(v6) => v6,
            };
            ppp::v2::IPv6::new(
                src_v6.octets(),
                dst_v6.octets(),
                source_addr.port(),
                dest_addr.port(),
            )
            .into()
        }
    };

    let version_command = ppp::v2::Version::Two as u8 | ppp::v2::Command::Proxy as u8;
    let header_bytes =
        ppp::v2::Builder::with_addresses(version_command, ppp::v2::Protocol::Stream, addresses)
            .build()
            .map_err(|e| TransportError::ProxyProtocolDecode(e.to_string()))?;

    stream
        .write_all(&header_bytes)
        .await
        .map_err(TransportError::Forward)?;

    Ok(())
}
