use std::{
    io,
    net::{IpAddr, ToSocketAddrs},
    time::SystemTime,
};

use crate::network::ProtocolError;
use crate::network::Result as ProtocolResult;
use crate::packet::{PacketCodec, Result as PacketResult};
use crate::types::{Byte, ProtocolRead, ProtocolString, ProtocolWrite, UnsignedShort, VarInt};

pub const SERVERBOUND_HANDSHAKE_ID: i32 = 0x00;

#[derive(Debug, Clone, PartialEq)]
pub struct ServerBoundHandshake {
    pub protocol_version: VarInt,
    pub server_address: ProtocolString,
    pub server_port: UnsignedShort,
    pub next_state: Byte,
}

const SEPARATOR_FORGE: &str = "\0";
const SEPARATOR_REAL_IP: &str = "///";

impl ServerBoundHandshake {
    pub const STATE_STATUS: i8 = 1;
    pub const STATE_LOGIN: i8 = 2;

    pub fn new(
        protocol_version: i32,
        server_address: String,
        server_port: u16,
        next_state: i8,
    ) -> Self {
        Self {
            protocol_version: VarInt(protocol_version),
            server_address: ProtocolString(server_address),
            server_port: UnsignedShort(server_port),
            next_state: Byte(next_state),
        }
    }

    pub fn is_status_request(&self) -> bool {
        self.next_state.0 == Self::STATE_STATUS
    }

    pub fn is_login_request(&self) -> bool {
        self.next_state.0 == Self::STATE_LOGIN
    }

    pub fn is_forge_address(&self) -> bool {
        self.server_address.0.contains(SEPARATOR_FORGE)
    }

    pub fn is_real_ip_address(&self) -> bool {
        self.server_address.0.contains(SEPARATOR_REAL_IP)
    }

    pub fn parse_server_address(&self) -> String {
        let addr = &self.server_address.0;
        let addr = match addr.find(SEPARATOR_FORGE) {
            Some(idx) => &addr[..idx],
            None => addr,
        };
        let addr = match addr.find(SEPARATOR_REAL_IP) {
            Some(idx) => &addr[..idx],
            None => addr,
        };
        addr.trim_matches('.').to_string()
    }

    pub fn parse_real_ip(&self) -> ProtocolResult<(String, SystemTime, IpAddr, u16)> {
        let parts: Vec<&str> = self.server_address.0.split(SEPARATOR_REAL_IP).collect();
        if parts.len() < 3 {
            return Err(ProtocolError::InvalidLength(parts.len()));
        }
        //["example.com", "127.0.0.1:25565", "0"]
        let addr = parts[0].to_string();
        let timestamp = match parts[2].parse::<u64>() {
            Ok(ts) => SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(ts),
            Err(_) => return Err(ProtocolError::InvalidLength(0)),
        };

        let addr_parts: Vec<&str> = parts[1].split(':').collect();
        if addr_parts.len() < 2 {
            return Err(ProtocolError::InvalidLength(addr_parts.len()));
        }

        let port = match addr_parts[1].parse::<u16>() {
            Ok(port) => port,
            Err(_) => return Err(ProtocolError::InvalidLength(0)),
        };

        let ip = match addr_parts[0].parse::<IpAddr>() {
            Ok(ip) => ip,
            Err(_) => return Err(ProtocolError::InvalidLength(0)),
        };

        Ok((addr, timestamp, ip, port))
    }

    pub fn upgrade_to_real_ip<A: ToSocketAddrs>(
        &mut self,
        client_addr: A,
        timestamp: SystemTime,
    ) -> ProtocolResult<()> {
        let addr = match client_addr.to_socket_addrs()?.next() {
            Some(addr) => addr,
            None => return Err(ProtocolError::InvalidLength(0)),
        };

        let unix_timestamp = match timestamp.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(n) => n.as_secs(),
            Err(_) => return Err(ProtocolError::InvalidLength(0)),
        };

        let addr_parts: Vec<&str> = self.server_address.0.split(SEPARATOR_FORGE).collect();
        let mut new_addr = format!(
            "{}{}{}{}{}",
            addr_parts[0], SEPARATOR_REAL_IP, addr, SEPARATOR_REAL_IP, unix_timestamp
        );

        if addr_parts.len() > 1 {
            new_addr = format!("{}{}{}", new_addr, "\0", addr_parts[1]);
        }

        self.server_address = ProtocolString(new_addr);
        Ok(())
    }

    /// Read a handshake packet from bytes
    pub fn read_from_bytes(data: &[u8]) -> io::Result<Self> {
        let mut reader = data;

        let protocol_version = VarInt::read_from(&mut reader)?;
        let server_address = ProtocolString::read_from(&mut reader)?;
        let server_port = UnsignedShort::read_from(&mut reader)?;
        let next_state = Byte::read_from(&mut reader)?;

        Ok(Self {
            protocol_version: protocol_version.0,
            server_address: server_address.0,
            server_port: server_port.0,
            next_state: next_state.0,
        })
    }

    /// Create a new packet for this handshake
    pub fn to_packet<P: PacketCodec>(&self, packet: &mut P) -> PacketResult<()> {
        packet.encode(&self.protocol_version)?;
        packet.encode(&self.server_address)?;
        packet.encode(&self.server_port)?;
        packet.encode(&VarInt(2))?;
        Ok(())
    }

    /// Decode a handshake from a packet
    pub fn from_packet<P: PacketCodec>(packet: &P) -> io::Result<Self> {
        if packet.id() != SERVERBOUND_HANDSHAKE_ID {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid packet ID for handshake",
            ));
        }

        Self::read_from_bytes(packet.data())
    }
}

impl ProtocolWrite for ServerBoundHandshake {
    fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let mut written = 0;
        written += self.protocol_version.write_to(writer)?;
        written += self.server_address.write_to(writer)?;
        written += self.server_port.write_to(writer)?;
        written += self.next_state.write_to(writer)?;
        Ok(written)
    }
}

impl ProtocolRead for ServerBoundHandshake {
    fn read_from<R: io::Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut bytes_read = 0;

        let (protocol_version, n) = VarInt::read_from(reader)?;
        bytes_read += n;

        let (server_address, n) = ProtocolString::read_from(reader)?;
        bytes_read += n;

        let (server_port, n) = UnsignedShort::read_from(reader)?;
        bytes_read += n;

        let (next_state, n) = Byte::read_from(reader)?;
        bytes_read += n;

        Ok((
            Self {
                protocol_version,
                server_address,
                server_port,
                next_state,
            },
            bytes_read,
        ))
    }
}
