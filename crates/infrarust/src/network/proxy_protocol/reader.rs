use std::{net::SocketAddr, time::Duration};

use tokio::{io::AsyncReadExt, net::TcpStream, time::timeout};
use tracing::{debug, error, warn};

use super::{ProtocolResult, errors::ProxyProtocolError};

static PROXY_PROTOCOL_V1_SIGNATURE: &[u8] = b"PROXY ";
static PROXY_PROTOCOL_V2_SIGNATURE: &[u8] = b"\x0D\x0A\x0D\x0A\x00\x0D\x0A\x51\x55\x49\x54\x0A";

#[derive(Debug, Clone)]
pub struct ProxyProtocolReader {
    /// Whether to expect and read a proxy protocol header
    pub enabled: bool,
    /// Maximum time to wait for the proxy protocol header
    pub timeout: Duration,
    /// Allow both v1 and v2 formats or just one specific version
    pub allowed_versions: Option<Vec<u8>>,
}

impl Default for ProxyProtocolReader {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout: Duration::from_secs(5),
            allowed_versions: None, // Allow any version
        }
    }
}

impl ProxyProtocolReader {
    pub fn new(enabled: bool, timeout_secs: u64, allowed_versions: Option<Vec<u8>>) -> Self {
        Self {
            enabled,
            timeout: Duration::from_secs(timeout_secs),
            allowed_versions,
        }
    }

    pub async fn read_header(&self, stream: &mut TcpStream) -> ProtocolResult<Option<SocketAddr>> {
        if !self.enabled {
            debug!("Proxy protocol reading disabled, skipping header read");
            return Ok(None);
        }

        debug!("Reading proxy protocol header");

        // Read first bytes to determine protocol version
        let mut peek_buf = [0u8; 12];
        let read_result = timeout(self.timeout, stream.peek(&mut peek_buf)).await;

        match read_result {
            Ok(Ok(n)) if n >= 5 => {
                if peek_buf.starts_with(PROXY_PROTOCOL_V1_SIGNATURE) {
                    debug!("Detected Proxy Protocol v1");
                    self.read_v1_header(stream).await
                } else if n >= 12 && peek_buf.starts_with(PROXY_PROTOCOL_V2_SIGNATURE) {
                    debug!("Detected Proxy Protocol v2");
                    self.read_v2_header(stream).await
                } else {
                    debug!("No proxy protocol header detected");
                    Ok(None)
                }
            }
            Ok(Ok(_)) => {
                debug!("Not enough bytes to determine proxy protocol version");
                Ok(None)
            }
            Ok(Err(e)) => {
                error!("Error peeking stream: {}", e);
                Err(ProxyProtocolError::Io(e.to_string()))
            }
            Err(_) => {
                warn!("Timeout waiting for proxy protocol header");
                Err(ProxyProtocolError::Other(
                    "Timeout waiting for proxy protocol header".to_string(),
                ))
            }
        }
    }

    async fn read_v1_header(&self, stream: &mut TcpStream) -> ProtocolResult<Option<SocketAddr>> {
        let mut header_bytes = Vec::with_capacity(108); // Max v1 header size
        let mut buf = [0u8; 1];
        let mut found_cr = false;

        while header_bytes.len() < 108 {
            if let Err(e) = stream.read_exact(&mut buf).await {
                return Err(ProxyProtocolError::Io(e.to_string()));
            }

            header_bytes.push(buf[0]);

            if found_cr && buf[0] == b'\n' {
                break;
            }

            found_cr = buf[0] == b'\r';
        }

        // Parse v1 header manually since we only need the source address
        let header_str = match std::str::from_utf8(&header_bytes) {
            Ok(s) => s,
            Err(e) => {
                error!("Invalid UTF-8 in proxy protocol header: {}", e);
                return Err(ProxyProtocolError::InvalidHeader(format!(
                    "Invalid UTF-8: {}",
                    e
                )));
            }
        };

        // Format: "PROXY TCP4 192.168.1.1 192.168.1.2 12345 443\r\n"
        let parts: Vec<&str> = header_str.split_whitespace().collect();

        if parts.len() < 6 {
            error!("Invalid proxy protocol v1 header format");
            return Err(ProxyProtocolError::InvalidHeader(
                "Invalid v1 format".to_string(),
            ));
        }

        if parts[0] != "PROXY" {
            error!("Invalid proxy protocol header, doesn't start with PROXY");
            return Err(ProxyProtocolError::InvalidHeader(
                "Missing PROXY prefix".to_string(),
            ));
        }

        let proto = parts[1];
        let src_addr = parts[2];
        let src_port = match parts[4].parse::<u16>() {
            Ok(p) => p,
            Err(e) => {
                error!("Invalid source port in proxy protocol: {}", e);
                return Err(ProxyProtocolError::InvalidHeader(format!(
                    "Invalid source port: {}",
                    e
                )));
            }
        };

        let addr = match proto {
            "TCP4" => match src_addr.parse() {
                Ok(ipv4) => Some(SocketAddr::new(std::net::IpAddr::V4(ipv4), src_port)),
                Err(e) => {
                    error!("Invalid IPv4 address in proxy protocol: {}", e);
                    return Err(ProxyProtocolError::InvalidHeader(format!(
                        "Invalid IPv4: {}",
                        e
                    )));
                }
            },
            "TCP6" => match src_addr.parse() {
                Ok(ipv6) => Some(SocketAddr::new(std::net::IpAddr::V6(ipv6), src_port)),
                Err(e) => {
                    error!("Invalid IPv6 address in proxy protocol: {}", e);
                    return Err(ProxyProtocolError::InvalidHeader(format!(
                        "Invalid IPv6: {}",
                        e
                    )));
                }
            },
            "UNKNOWN" => None,
            _ => {
                error!("Unknown protocol family in proxy protocol: {}", proto);
                return Err(ProxyProtocolError::InvalidHeader(format!(
                    "Unknown protocol: {}",
                    proto
                )));
            }
        };

        debug!("Parsed proxy protocol v1, client addr: {:?}", addr);
        Ok(addr)
    }

    async fn read_v2_header(&self, stream: &mut TcpStream) -> ProtocolResult<Option<SocketAddr>> {
        // Read the 16-byte signature + ver_cmd + fam + len (first 16 bytes)
        let mut header = [0u8; 16];
        if let Err(e) = stream.read_exact(&mut header).await {
            return Err(ProxyProtocolError::Io(e.to_string()));
        }

        // Verify the signature
        if !header[0..12].eq(PROXY_PROTOCOL_V2_SIGNATURE) {
            return Err(ProxyProtocolError::InvalidHeader(
                "Invalid v2 signature".to_string(),
            ));
        }

        // Get the length from header bytes 14-15 (big-endian)
        let addr_len = ((header[14] as u16) << 8) | (header[15] as u16);
        let mut addr_data = vec![0u8; addr_len as usize];
        if let Err(e) = stream.read_exact(&mut addr_data).await {
            return Err(ProxyProtocolError::Io(e.to_string()));
        }

        let family = header[13] & 0xF0; // Upper 4 bits of 4th byte
        match family {
            // AF_INET: IPv4
            0x10 => {
                if addr_data.len() >= 12 {
                    // 4 bytes src addr + 4 bytes dst addr + 2 bytes src port + 2 bytes dst port
                    let mut src_ip = [0u8; 4];
                    src_ip.copy_from_slice(&addr_data[0..4]);

                    let src_port = ((addr_data[8] as u16) << 8) | addr_data[9] as u16;
                    let addr = SocketAddr::new(
                        std::net::IpAddr::V4(std::net::Ipv4Addr::from(src_ip)),
                        src_port,
                    );

                    debug!("Parsed proxy protocol v2 IPv4, client addr: {}", addr);
                    Ok(Some(addr))
                } else {
                    error!("IPv4 address data too short: {}", addr_data.len());
                    Err(ProxyProtocolError::InvalidLength(addr_data.len()))
                }
            }
            // AF_INET6: IPv6
            0x20 => {
                if addr_data.len() >= 36 {
                    // 16 bytes src addr + 16 bytes dst addr + 2 bytes src port + 2 bytes dst port
                    let mut src_ip = [0u8; 16];
                    src_ip.copy_from_slice(&addr_data[0..16]);

                    let src_port = ((addr_data[32] as u16) << 8) | addr_data[33] as u16;
                    let addr = SocketAddr::new(
                        std::net::IpAddr::V6(std::net::Ipv6Addr::from(src_ip)),
                        src_port,
                    );

                    debug!("Parsed proxy protocol v2 IPv6, client addr: {}", addr);
                    Ok(Some(addr))
                } else {
                    error!("IPv6 address data too short: {}", addr_data.len());
                    Err(ProxyProtocolError::InvalidLength(addr_data.len()))
                }
            }
            // AF_UNIX: Unix domain socket (not supported for now)
            0x30 => {
                debug!("Proxy protocol v2 with Unix domain socket, not extracting address");
                Ok(None)
            }
            // Unspecified/unknown
            0x00 => {
                debug!("Proxy protocol v2 with unspecified address family");
                Ok(None)
            }
            _ => {
                error!("Unknown address family in proxy protocol v2: {:#x}", family);
                Err(ProxyProtocolError::InvalidHeader(format!(
                    "Unknown family: {:#x}",
                    family
                )))
            }
        }
    }
}
