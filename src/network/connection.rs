use std::time::Duration;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::{
    io::{self, BufReader, BufWriter},
    net::TcpStream,
};
use tracing::warn;
use uuid::Uuid;

use crate::telemetry::{Direction, TELEMETRY};
use crate::EncryptionState;

use super::packet::io::RawPacketIO;
use super::packet::{Packet, PacketReader, PacketResult, PacketWriter};
use super::proxy_protocol::ProtocolResult;

pub enum PossibleReadValue {
    Packet(Packet),
    Raw(Vec<u8>),
}

pub struct Connection {
    reader: PacketReader<BufReader<OwnedReadHalf>>,
    writer: PacketWriter<BufWriter<OwnedWriteHalf>>,
    timeout: Duration,
    raw_mode: bool,
    pub session_id: Uuid,
}

impl Connection {
    pub async fn new(stream: TcpStream, session_id: Uuid) -> io::Result<Self> {
        match stream.set_nodelay(true) {
            Ok(_) => {}
            Err(e) => {
                TELEMETRY.record_internal_error("nodelay_failed", None, None);
                warn!("Failed to set nodelay: {}", e);
            }
        }

        let (read_half, write_half) = stream.into_split();

        Ok(Self {
            reader: PacketReader::new(BufReader::new(read_half)),
            writer: PacketWriter::new(BufWriter::new(write_half)),
            timeout: Duration::from_secs(30),
            raw_mode: false,
            session_id,
        })
    }

    pub fn enable_raw_mode(&mut self) {
        self.raw_mode = true;
    }

    pub async fn connect<A: tokio::net::ToSocketAddrs>(
        addr: A,
        session_id: Uuid,
    ) -> io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        Self::new(stream, session_id).await
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    pub fn enable_encryption(&mut self, encryption_state: &EncryptionState) {
        match encryption_state.create_cipher() {
            Some((encrypt, decrypt)) => {
                self.reader.enable_encryption(decrypt);
                self.writer.enable_encryption(encrypt);
            }
            None => {
                TELEMETRY.record_protocol_error("encryption_failed", "", self.session_id);
                warn!("Failed to enable encryption");
            }
        }
    }

    pub fn enable_compression(&mut self, threshold: i32) {
        self.reader.enable_compression(threshold);
        self.writer.enable_compression(threshold);
    }

    pub fn disable_compression(&mut self) {
        self.reader.disable_compression();
        self.writer.disable_compression();
    }

    pub fn is_compressing(&self) -> bool {
        self.reader.is_compressing()
    }

    pub async fn read_packet(&mut self) -> ProtocolResult<Packet> {
        Ok(self.reader.read_packet().await?)
    }

    pub async fn write_packet(&mut self, packet: &Packet) -> ProtocolResult<()> {
        TELEMETRY.record_bytes_transferred(Direction::Outgoing, packet.data.len() as u64, self.session_id);
        TELEMETRY.record_packet_processing(
            &format!("0x{:02x}", &packet.id),
            0.,
            self.session_id,
        );
        

        self.writer.write_packet(packet).await
    }

    pub async fn write_raw(&mut self, data: &[u8]) -> ProtocolResult<()> {
        TELEMETRY.record_bytes_transferred(Direction::Outgoing, data.len() as u64, self.session_id);
        Ok(self.writer.write_raw(data).await?)
    }

    pub async fn write(&mut self, data: PossibleReadValue) -> ProtocolResult<()> {
        match data {
            PossibleReadValue::Packet(packet) => self.write_packet(&packet).await?,
            PossibleReadValue::Raw(data) => self.write_raw(&data).await?,
        }
        Ok(())
    }

    pub async fn read(&mut self) -> PacketResult<PossibleReadValue> {
        if self.raw_mode {
            let data = match self.reader.read_raw().await? {
                Some(data) => {
                    let len = data.len();
                    TELEMETRY.record_bytes_transferred(
                        Direction::Incoming,
                        len as u64,
                        self.session_id,
                    );
                    data
                }
                None => {
                    let len = 0;
                    TELEMETRY.record_bytes_transferred(
                        Direction::Incoming,
                        len as u64,
                        self.session_id,
                    );
                    return Ok(PossibleReadValue::Raw(Vec::new()));
                }
            };

            Ok(PossibleReadValue::Raw(data.into()))
        } else {
            let packet = self.reader.read_packet().await?;
            let len = packet.data.len();
            TELEMETRY.record_bytes_transferred(Direction::Incoming, len as u64, self.session_id);
            Ok(PossibleReadValue::Packet(packet))
        }
    }

    pub async fn close(&mut self) -> PacketResult<()> {
        self.writer.close().await
    }

    pub async fn peer_addr(&self) -> PacketResult<std::net::SocketAddr> {
        self.reader
            .reader
            .get_ref()
            .peer_addr()
            .map_err(|e| e.into())
    }

    pub fn into_split(
        self,
    ) -> (
        PacketReader<BufReader<tokio::net::tcp::OwnedReadHalf>>,
        PacketWriter<BufWriter<tokio::net::tcp::OwnedWriteHalf>>,
    ) {
        let reader = self.reader;
        let writer = self.writer;
        (reader, writer)
    }

    pub fn into_split_raw(
        self,
    ) -> (
        PacketReader<BufReader<OwnedReadHalf>>,
        PacketWriter<BufWriter<OwnedWriteHalf>>,
    ) {
        (self.reader, self.writer)
    }
}

pub struct ServerConnection {
    connection: Connection,
    pub session_id: Uuid,
}

impl ServerConnection {
    pub async fn new(stream: TcpStream, session_id: Uuid) -> io::Result<Self> {
        Ok(Self {
            connection: Connection::new(stream, session_id).await?,
            session_id,
        })
    }

    pub async fn connect<A: tokio::net::ToSocketAddrs>(
        addr: A,
        session_id: Uuid,
    ) -> io::Result<Self> {
        Ok(Self {
            connection: Connection::connect(addr, session_id).await?,
            session_id,
        })
    }

    pub fn into_split(
        self,
    ) -> (
        PacketReader<BufReader<tokio::net::tcp::OwnedReadHalf>>,
        PacketWriter<BufWriter<tokio::net::tcp::OwnedWriteHalf>>,
    ) {
        self.connection.into_split()
    }

    pub fn into_split_raw(
        self,
    ) -> (
        PacketReader<BufReader<tokio::net::tcp::OwnedReadHalf>>,
        PacketWriter<BufWriter<tokio::net::tcp::OwnedWriteHalf>>,
    ) {
        self.connection.into_split_raw()
    }

    pub async fn close(&mut self) -> PacketResult<()> {
        self.connection.close().await
    }

    pub async fn peer_addr(&self) -> PacketResult<std::net::SocketAddr> {
        self.connection.peer_addr().await
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.connection.set_timeout(timeout);
    }

    pub fn enable_encryption(&mut self, encryption_state: &EncryptionState) {
        self.connection.enable_encryption(encryption_state);
    }

    pub fn enable_compression(&mut self, threshold: i32) {
        self.connection.enable_compression(threshold);
    }

    pub fn disable_compression(&mut self) {
        self.connection.disable_compression();
    }

    pub fn is_compressing(&self) -> bool {
        self.connection.is_compressing()
    }

    pub async fn read_packet(&mut self) -> ProtocolResult<Packet> {
        self.connection.read_packet().await
    }

    pub async fn write_packet(&mut self, packet: &Packet) -> ProtocolResult<()> {
        self.connection.write_packet(packet).await
    }

    pub async fn write_raw(&mut self, data: &[u8]) -> ProtocolResult<()> {
        self.connection.write_raw(data).await
    }

    pub async fn write(&mut self, data: PossibleReadValue) -> ProtocolResult<()> {
        self.connection.write(data).await
    }

    pub async fn read(&mut self) -> PacketResult<PossibleReadValue> {
        self.connection.read().await
    }

    pub fn enable_raw_mode(&mut self) {
        self.connection.enable_raw_mode();
    }
}
