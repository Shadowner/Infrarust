use super::ProxyModeHandler;
use crate::network::{connection::Connection, packet::io::RawPacketIO};
use crate::server::ServerResponse;
use crate::version::Version;
use async_trait::async_trait;
use log::{debug, error};
use std::io::{self};

pub struct PassthroughMode;

#[async_trait]
impl ProxyModeHandler for PassthroughMode {
    async fn handle(
        &self,
        client: Connection,
        response: ServerResponse,
        protocol_version: Version,
    ) -> io::Result<()> {
        if let Some(_addr) = response.server_addr {
            let server = response.server_conn;

            let (mut client_read, mut client_write) = client.into_split_raw();
            let (mut server_read, mut server_write) = server.unwrap().into_split_raw();
            client_read.set_protocol_version(protocol_version);
            server_read.set_protocol_version(protocol_version);

            // Forward initial handshake packets
            debug!("Forwarding initial handshake packets in passthrough mode");
            for packet in response.read_packets {
                debug!("Client -> Server: Packet ID: 0x{:02x}", packet.id);
                match packet.into_raw() {
                    Ok(data) => server_write.write_raw(&data).await?,
                    Err(e) => {
                        error!("Failed to convert packet to raw data: {}", e);
                        continue;
                    }
                }
            }

            debug!("=== Login sequence completed, entering play phase ===");

            loop {
                tokio::select! {
                    result = client_read.read_raw() => {
                        match result {
                            Ok(Some(data)) => {
                                debug!("Client -> Server: Raw data length: {}", data.len());
                                if let Err(e) = server_write.write_raw(&data).await {
                                    error!("Failed to write to server: {}", e);
                                    break;
                                }
                            }
                            Ok(None) => {
                                debug!("Client disconnected cleanly");
                                break;
                            }
                            Err(e) => {
                                error!("Client read error: {}", e);
                                break;
                            }
                        }
                    }
                    result = server_read.read_raw() => {
                        match result {
                            Ok(Some(data)) => {
                                debug!("Server -> Client: Raw data length: {}", data.len());
                                if let Err(e) = client_write.write_raw(&data).await {
                                    error!("Failed to write to client: {}", e);
                                    break;
                                }
                            }
                            Ok(None) => {
                                debug!("Server disconnected cleanly");
                                break;
                            }
                            Err(e) => {
                                error!("Server read error: {}", e);
                                break;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
