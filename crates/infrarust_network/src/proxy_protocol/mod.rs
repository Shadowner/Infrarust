use std::{io, net::SocketAddr, str::FromStr};

use errors::ProxyProtocolError;
use ipnetwork::IpNetwork;
use tokio::{io::AsyncWriteExt, net::TcpStream};

pub mod errors;

pub type ProtocolResult<T> = Result<T, ProxyProtocolError>;

#[derive(Clone)]
pub struct ProxyProtocolConfig {
    pub receive: bool,
    pub trusted_cidrs: Vec<IpNetwork>,
}

// Implement a real ProxyProtocolConfig

impl ProxyProtocolConfig {
    pub fn new(trusted_cidrs: Vec<String>) -> Result<Self, ProxyProtocolError> {
        if trusted_cidrs.is_empty() {
            return Err(ProxyProtocolError::NoTrustedCIDRs);
        }

        let trusted_networks = trusted_cidrs
            .iter()
            .map(|cidr| IpNetwork::from_str(cidr))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| ProxyProtocolError::Io(io::Error::new(io::ErrorKind::InvalidInput, e)))?;

        Ok(Self {
            receive: true,
            trusted_cidrs: trusted_networks,
        })
    }

    pub fn is_trusted(&self, addr: &SocketAddr) -> bool {
        self.trusted_cidrs
            .iter()
            .any(|cidr| cidr.contains(addr.ip()))
    }
}

pub async fn write_proxy_protocol_header(
    client_addr: SocketAddr,
    server: &mut TcpStream,
) -> io::Result<()> {
    // Create the PROXY protocol v2 header
    let header = b"\x0D\x0A\x0D\x0A\x00\x0D\x0A\x51\x55\x49\x54\x0A"; // Signature
    let version_command = b"\x21"; // Version 2, PROXY command
    let family = if client_addr.is_ipv4() {
        b"\x11"
    } else {
        b"\x12"
    }; // TCP/IPv4 or TCP/IPv6

    // Write the header parts
    server.write_all(header).await?;
    server.write_all(version_command).await?;
    server.write_all(family).await?;

    // For now, just write some placeholder address data
    let addr_data = [0u8; 16]; // Placeholder for address data
    server.write_all(&addr_data).await?;

    server.flush().await?;
    Ok(())
}
