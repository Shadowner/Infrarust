use std::{io, net::SocketAddr, str::FromStr};

use bytes::BytesMut;
use errors::ProxyProtocolError;
use ipnetwork::IpNetwork;
use proxy_protocol::{encode, version1::ProxyAddresses, ProxyHeader};
use tokio::{io::AsyncWriteExt, net::TcpStream};

pub mod errors;

pub type ProtocolResult<T> = Result<T, ProxyProtocolError>;

#[derive(Clone, Debug, Default)]
pub struct ProxyProtocolConfig {
    pub enabled: bool,
    pub version: Option<u8>, // 1 pour v1, 2 pour v2
}pub async fn write_proxy_protocol_header(
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
    use proxy_protocol::version1::ProxyAddresses;
    use proxy_protocol::{ProxyHeader, encode};
    
    let addresses = match (client_addr, server_addr) {
        (SocketAddr::V4(source), SocketAddr::V4(destination)) => {
            ProxyAddresses::Ipv4 { source, destination }
        },
        (SocketAddr::V6(source), SocketAddr::V6(destination)) => {
            ProxyAddresses::Ipv6 { source, destination }
        },
        _ => ProxyAddresses::Unknown,
    };

    let header = ProxyHeader::Version1 { addresses };
    
    encode(header).map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))
}

fn create_v2_header(client_addr: SocketAddr, server_addr: SocketAddr) -> io::Result<BytesMut> {
    use proxy_protocol::version2::{ProxyAddresses, ProxyCommand, ProxyTransportProtocol};
    use proxy_protocol::{ProxyHeader, encode};
    
    let (addresses, transport_protocol) = match (client_addr, server_addr) {
        (SocketAddr::V4(source), SocketAddr::V4(destination)) => {
            (ProxyAddresses::Ipv4 { source, destination }, ProxyTransportProtocol::Stream)
        },
        (SocketAddr::V6(source), SocketAddr::V6(destination)) => {
            (ProxyAddresses::Ipv6 { source, destination }, ProxyTransportProtocol::Stream)
        },
        _ => (ProxyAddresses::Unspec, ProxyTransportProtocol::Unspec),
    };

    let header = ProxyHeader::Version2 { 
        command: ProxyCommand::Proxy,
        transport_protocol,
        addresses,
    };
    
    encode(header).map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))
}