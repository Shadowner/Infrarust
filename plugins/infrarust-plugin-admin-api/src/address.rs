use std::net::SocketAddr;

use infrarust_api::types::ServerAddress;

use crate::error::ApiError;

/// Renders an address so that [`parse_address`] can read it back, which
/// `ServerAddress`'s own `host:port` form cannot do for IPv6.
pub fn format_address(addr: &ServerAddress) -> String {
    if addr.host.contains(':') {
        format!("[{}]:{}", addr.host, addr.port)
    } else {
        format!("{}:{}", addr.host, addr.port)
    }
}

pub fn parse_address(raw: &str) -> Result<ServerAddress, ApiError> {
    let raw = raw.trim();
    if let Ok(socket) = raw.parse::<SocketAddr>() {
        return Ok(ServerAddress {
            host: socket.ip().to_string(),
            port: socket.port(),
        });
    }

    let invalid = || ApiError::BadRequest(format!("Invalid backend address: {raw}"));
    let (host, port) = raw.rsplit_once(':').ok_or_else(invalid)?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return Err(invalid());
    }
    Ok(ServerAddress {
        host: host.to_string(),
        port: port.parse().map_err(|_| invalid())?,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn address_round_trips() {
        for raw in ["10.0.0.1:25565", "[::1]:25565", "mc.example.com:25566"] {
            assert_eq!(format_address(&parse_address(raw).unwrap()), raw);
        }
    }

    #[test]
    fn address_rejects_garbage() {
        assert!(parse_address("no-port").is_err());
        assert!(parse_address(":25565").is_err());
        assert!(parse_address("host:not-a-port").is_err());
    }
}
