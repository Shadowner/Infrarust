use std::{
    io::{self, Error, ErrorKind},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncWriteExt, BufReader, BufWriter},
    net::{
        TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
};
use tracing::warn;
use uuid::Uuid;

use crate::EncryptionState;

#[cfg(feature = "telemetry")]
use crate::telemetry::{Direction, TELEMETRY};

use super::{
    packet::{Packet, PacketError, PacketReader, PacketResult, PacketWriter, io::RawPacketIO},
    proxy_protocol::ProtocolResult,
};

#[derive(Debug, Clone)]
pub enum PossibleReadValue {
    Raw(Vec<u8>),
    Packet(Packet),
    Nothing,
    Eof,
}

impl PossibleReadValue {
    pub fn get_type(&self) -> &'static str {
        match self {
            PossibleReadValue::Packet(_) => "Packet",
            PossibleReadValue::Raw(_) => "Raw",
            PossibleReadValue::Nothing => "Nothing",
            PossibleReadValue::Eof => "Eof",
        }
    }
}

#[derive(Debug)]
enum ConnectionMode {
    Protocol,
    Raw,
}

#[derive(Debug)]
pub struct Connection {
    reader: PacketReader<BufReader<OwnedReadHalf>>,
    writer: PacketWriter<BufWriter<OwnedWriteHalf>>,
    pub session_id: Uuid,
    mode: ConnectionMode,
    closed: Arc<AtomicBool>,
    timeout: Duration,
}

impl Connection {
    pub async fn new(stream: TcpStream, session_id: Uuid) -> io::Result<Self> {
        let (read_half, write_half) = stream.into_split();

        Ok(Self {
            reader: PacketReader::new(BufReader::new(read_half)),
            writer: PacketWriter::new(BufWriter::new(write_half)),
            session_id,
            mode: ConnectionMode::Protocol,
            closed: Arc::new(AtomicBool::new(false)),
            timeout: Duration::from_secs(30),
        })
    }

    pub fn enable_raw_mode(&mut self) {
        self.mode = ConnectionMode::Raw;
    }

    pub async fn read(&mut self) -> io::Result<PossibleReadValue> {
        // If connection is already closed don't try to read
        if self.closed.load(Ordering::SeqCst) {
            return Err(Error::new(
                ErrorKind::ConnectionAborted,
                "Connection already closed",
            ));
        }

        match self.mode {
            ConnectionMode::Protocol => match self.reader.read_packet().await {
                Ok(packet) => Ok(PossibleReadValue::Packet(packet)),
                Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                    self.closed.store(true, Ordering::SeqCst);
                    Ok(PossibleReadValue::Eof)
                }
                Err(e) => Err(e.into()),
            },
            ConnectionMode::Raw => {
                match self.reader.read_raw().await {
                    Ok(None) => {
                        // EOF - mark connection as closed
                        self.closed.store(true, Ordering::SeqCst);
                        Ok(PossibleReadValue::Eof)
                    }
                    Ok(Some(bytes)) => Ok(PossibleReadValue::Raw(bytes.to_vec())),
                    Err(e) => {
                        // Mark connection as closed on error
                        self.closed.store(true, Ordering::SeqCst);
                        Err(e.into())
                    }
                }
            }
        }
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
                #[cfg(feature = "telemetry")]
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
        #[cfg(feature = "telemetry")]
        TELEMETRY.record_bytes_transferred(
            Direction::Outgoing,
            packet.data.len() as u64,
            self.session_id,
        );

        #[cfg(feature = "telemetry")]
        TELEMETRY.record_packet_processing(&format!("0x{:02x}", &packet.id), 0., self.session_id);

        self.writer.write_packet(packet).await
    }

    pub async fn write_raw(&mut self, data: &[u8]) -> ProtocolResult<()> {
        #[cfg(feature = "telemetry")]
        TELEMETRY.record_bytes_transferred(Direction::Outgoing, data.len() as u64, self.session_id);

        Ok(self.writer.write_raw(data).await?)
    }

    pub async fn write(&mut self, data: PossibleReadValue) -> ProtocolResult<()> {
        match data {
            PossibleReadValue::Packet(packet) => self.write_packet(&packet).await?,
            PossibleReadValue::Raw(data) => self.write_raw(&data).await?,
            PossibleReadValue::Eof => {
                self.close().await?;
                return Err(
                    io::Error::new(io::ErrorKind::UnexpectedEof, "Connection closed").into(),
                );
            }
            PossibleReadValue::Nothing => {}
        }
        Ok(())
    }

    pub async fn peer_addr(&self) -> PacketResult<std::net::SocketAddr> {
        self.reader
            .reader
            .get_ref()
            .peer_addr()
            .map_err(|e| e.into())
    }

    pub async fn close(&mut self) -> io::Result<()> {
        // Avoid double-closing
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        // Shutdown the writer - this should trigger EOF on the remote end
        self.writer.flush().await?;
        self.writer.get_mut().shutdown().await?;

        // For the reader, we can't do much other than mark it as closed
        // The next read will return an error

        Ok(())
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

#[derive(Debug)]
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
        self.connection.close().await.map_err(PacketError::from)
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

    pub async fn read(&mut self) -> io::Result<PossibleReadValue> {
        self.connection.read().await
    }

    pub fn enable_raw_mode(&mut self) {
        self.connection.enable_raw_mode();
    }
}
