use std::time::Duration;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::{
    io::{self, BufReader, BufWriter},
    net::TcpStream,
};

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
}

impl Connection {
    pub async fn new(stream: TcpStream) -> io::Result<Self> {
        stream.set_nodelay(true)?;
        let (read_half, write_half) = stream.into_split();

        Ok(Self {
            reader: PacketReader::new(BufReader::new(read_half)),
            writer: PacketWriter::new(BufWriter::new(write_half)),
            timeout: Duration::from_secs(30),
            raw_mode: false,
        })
    }

    pub async fn enable_raw_mode(&mut self) {
        self.raw_mode = true;
    }

    pub async fn connect<A: tokio::net::ToSocketAddrs>(addr: A) -> io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        Self::new(stream).await
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    pub fn enable_encryption(&mut self, encryption_state: &EncryptionState) {
        if let Some((encrypt, decrypt)) = encryption_state.create_cipher() {
            self.reader.enable_encryption(decrypt);
            self.writer.enable_encryption(encrypt);
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

    pub async fn read_packet(&mut self) -> ProtocolResult<Packet> {
        Ok(self.reader.read_packet().await?)
    }

    pub async fn write_packet(&mut self, packet: &Packet) -> ProtocolResult<()> {
        self.writer.write_packet(packet).await
    }

    pub async fn write_raw(&mut self, data: &[u8]) -> ProtocolResult<()> {
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
                Some(data) => data,
                None => return Ok(PossibleReadValue::Raw(Vec::new())),
            };

            Ok(PossibleReadValue::Raw(data.into()))
        } else {
            let packet = self.reader.read_packet().await?;
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
}

impl ServerConnection {
    pub async fn new(stream: TcpStream) -> io::Result<Self> {
        Ok(Self {
            connection: Connection::new(stream).await?,
        })
    }

    pub async fn connect<A: tokio::net::ToSocketAddrs>(addr: A) -> io::Result<Self> {
        Ok(Self {
            connection: Connection::connect(addr).await?,
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
}
