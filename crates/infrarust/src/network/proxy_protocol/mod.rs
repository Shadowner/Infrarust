use std::{io, net::SocketAddr};

use bytes::BytesMut;
use errors::ProxyProtocolError;
use infrarust_config::models::infrarust::ProxyProtocolConfig;
use tokio::{io::AsyncWriteExt, net::TcpStream};

pub mod errors;
pub mod reader;

pub type ProtocolResult<T> = Result<T, ProxyProtocolError>;

pub async fn write_proxy_protocol_header(
    stream: &mut TcpStream,
    client_addr: SocketAddr,
    server_addr: SocketAddr,
    config: &ProxyProtocolConfig,
) -> io::Result<()> {
    if !config.enabled {
        return Ok(());
    }

    let header_bytes = match config.version.unwrap_or(1) {
        1 => create_v1_header(client_addr, server_addr)?,
        2 => create_v2_header(client_addr, server_addr)?,
        _ => create_v1_header(client_addr, server_addr)?, // default v1
    };

    stream.write_all(&header_bytes).await?;

    Ok(())
}

fn create_v1_header(client_addr: SocketAddr, server_addr: SocketAddr) -> io::Result<BytesMut> {
    use proxy_protocol::ProxyHeader;
    use proxy_protocol::version1::ProxyAddresses;

    let addresses = match (client_addr, server_addr) {
        (SocketAddr::V4(source), SocketAddr::V4(destination)) => ProxyAddresses::Ipv4 {
            source,
            destination,
        },
        (SocketAddr::V6(source), SocketAddr::V6(destination)) => ProxyAddresses::Ipv6 {
            source,
            destination,
        },
        _ => ProxyAddresses::Unknown,
    };

    let header = ProxyHeader::Version1 { addresses };

    proxy_protocol::encode(header).map_err(|e| io::Error::other(e.to_string()))
}

fn create_v2_header(client_addr: SocketAddr, server_addr: SocketAddr) -> io::Result<BytesMut> {
    use proxy_protocol::ProxyHeader;
    use proxy_protocol::version2::{ProxyAddresses, ProxyCommand, ProxyTransportProtocol};

    let (addresses, transport_protocol) = match (client_addr, server_addr) {
        (SocketAddr::V4(source), SocketAddr::V4(destination)) => (
            ProxyAddresses::Ipv4 {
                source,
                destination,
            },
            ProxyTransportProtocol::Stream,
        ),
        (SocketAddr::V6(source), SocketAddr::V6(destination)) => (
            ProxyAddresses::Ipv6 {
                source,
                destination,
            },
            ProxyTransportProtocol::Stream,
        ),
        _ => (ProxyAddresses::Unspec, ProxyTransportProtocol::Unspec),
    };

    let header = ProxyHeader::Version2 {
        command: ProxyCommand::Proxy,
        transport_protocol,
        addresses,
    };

    proxy_protocol::encode(header).map_err(|e| io::Error::other(e.to_string()))
}
