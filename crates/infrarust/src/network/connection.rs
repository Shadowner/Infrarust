use std::{
    io::{self, Error, ErrorKind},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use bytes::BytesMut;
use infrarust_config::models::infrarust::ProxyProtocolConfig;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter},
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
    Raw(BytesMut),
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
    /// if received via proxy protocol
    pub original_client_addr: Option<std::net::SocketAddr>,
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
            original_client_addr: None,
        })
    }

    pub async fn with_proxy_protocol(
        mut stream: TcpStream,
        session_id: Uuid,
        proxy_config: Option<&ProxyProtocolConfig>,
    ) -> io::Result<Self> {
        use crate::network::proxy_protocol::reader::ProxyProtocolReader;

        let mut original_client_addr = None;

        if let Some(config) = proxy_config
            && config.receive_enabled
        {
            let reader = ProxyProtocolReader::new(
                config.receive_enabled,
                config.receive_timeout_secs.unwrap_or(5),
                config.receive_allowed_versions.clone(),
            );

            match reader.read_header(&mut stream).await {
                Ok(addr) => {
                    original_client_addr = addr;
                }
                Err(e) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Failed to read proxy protocol header: {}", e),
                    ));
                }
            }
        }

        let (read_half, write_half) = stream.into_split();

        Ok(Self {
            reader: PacketReader::new(BufReader::new(read_half)),
            writer: PacketWriter::new(BufWriter::new(write_half)),
            session_id,
            mode: ConnectionMode::Protocol,
            closed: Arc::new(AtomicBool::new(false)),
            timeout: Duration::from_secs(30),
            original_client_addr,
        })
    }

    pub fn enable_raw_mode(&mut self) {
        self.mode = ConnectionMode::Raw;
    }

    pub async fn peek_first_byte(&mut self) -> io::Result<u8> {
        let buf = self.reader.reader.fill_buf().await?;
        if buf.is_empty() {
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                "Connection closed before any data received",
            ));
        }
        Ok(buf[0])
    }

    pub async fn read_exact_raw(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.reader.reader.read_exact(buf).await?;
        Ok(())
    }

    pub async fn read_raw_up_to(&mut self, max_len: usize) -> io::Result<Vec<u8>> {
        let mut result = Vec::with_capacity(max_len);
        let mut remaining = max_len;

        while remaining > 0 {
            let buf = self.reader.reader.fill_buf().await?;
            if buf.is_empty() {
                break;
            }
            let to_read = buf.len().min(remaining);
            result.extend_from_slice(&buf[..to_read]);
            self.reader.reader.consume(to_read);
            remaining -= to_read;
        }

        Ok(result)
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
                    Ok(Some(bytes)) => Ok(PossibleReadValue::Raw(bytes)),
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

    pub async fn flush(&mut self) -> PacketResult<()> {
        self.writer.flush().await
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
    pub fn into_tcp_stream(self) -> io::Result<TcpStream> {
        if !self.reader.buffer().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Buffered read data would be lost",
            ));
        }
        let buf_reader = self.reader.into_inner();
        let buf_writer = self.writer.into_inner();

        if !buf_reader.buffer().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "BufReader has buffered data that would be lost",
            ));
        }
        let read_half = buf_reader.into_inner();
        let write_half = buf_writer.into_inner();
        read_half
            .reunite(write_half)
            .map_err(|e| io::Error::other(e.to_string()))
    }
    pub async fn into_tcp_stream_async(mut self) -> io::Result<TcpStream> {
        self.writer
            .flush()
            .await
            .map_err(|e| io::Error::other(format!("Flush failed: {}", e)))?;
        if !self.reader.buffer().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Buffered read data would be lost",
            ));
        }

        let buf_reader = self.reader.into_inner();
        let buf_writer = self.writer.into_inner();

        if !buf_reader.buffer().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "BufReader has buffered data that would be lost",
            ));
        }

        let read_half = buf_reader.into_inner();
        let write_half = buf_writer.into_inner();

        read_half
            .reunite(write_half)
            .map_err(|e| io::Error::other(e.to_string()))
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

    pub async fn with_proxy_protocol(
        stream: TcpStream,
        session_id: Uuid,
        proxy_config: Option<&ProxyProtocolConfig>,
    ) -> io::Result<Self> {
        Ok(Self {
            connection: Connection::with_proxy_protocol(stream, session_id, proxy_config).await?,
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

    pub fn into_tcp_stream(self) -> io::Result<TcpStream> {
        self.connection.into_tcp_stream()
    }

    pub async fn into_tcp_stream_async(self) -> io::Result<TcpStream> {
        self.connection.into_tcp_stream_async().await
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

    pub async fn flush(&mut self) -> ProtocolResult<()> {
        self.connection.flush().await.map_err(Into::into)
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

    pub async fn read_exact_raw(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.connection.read_exact_raw(buf).await
    }

    pub async fn read_raw_up_to(&mut self, max_len: usize) -> io::Result<Vec<u8>> {
        self.connection.read_raw_up_to(max_len).await
    }
}
