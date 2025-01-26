use super::ProxyModeHandler;
use crate::{
    network::{connection::Connection, packet::PacketCodec},
    protocol::types::VarInt,
    version::Version,
};
use async_trait::async_trait;
use log::{debug, error};
use std::io;

/// First attempt at a proxy before realising that Yggdrasil authentication
/// is not possible for a full online mode client -> proxy -> server
pub struct FullMode;

#[async_trait]
impl ProxyModeHandler for FullMode {
    async fn handle(
        &self,
        client: Connection,
        response: crate::server::ServerResponse,
        proxy_protocol: Version,
    ) -> io::Result<()> {
        if let Some(_addr) = response.server_addr {
            let mut server = response.server_conn.unwrap();
            let mut encryption_started = false;

            debug!("Forwarding initial handshake packets in full mode");
            for packet in response.read_packets {
                server.write_packet(&packet).await?;
            }

            let (mut client_read, mut client_write) = client.into_split_raw();
            let (mut server_read, mut server_write) = server.into_split_raw();

            client_read.set_protocol_version(proxy_protocol);
            server_read.set_protocol_version(proxy_protocol);

            loop {
                tokio::select! {
                    result = client_read.read_packet() => {
                        match result {
                            Ok(packet) => {
                                if !encryption_started && packet.id == 0x01 {  // Encryption Response
                                    encryption_started = true;
                                    debug!("Full mode: encryption started (client)");
                                }
                                debug!("Client -> Server: Packet ID: 0x{:02x}", packet.id);
                                server_write.write_packet(&packet).await?;
                            }
                            Err(e) => {
                                error!("Client read error: {}", e);
                                break;
                            }
                        }
                    },
                    result = server_read.read_packet() => {
                        match result {
                            Ok(packet) => {
                                match packet.id {
                                    0x01 => {  // Encryption Request
                                        encryption_started = true;
                                        debug!("Full mode: encryption started (server)");
                                    }
                                    0x03 => {  // Set Compression
                                        if let Ok(threshold) = packet.decode::<VarInt>() {
                                            if threshold.0 >= 0 {
                                                debug!("Enabling compression with threshold {}", threshold.0);
                                                client_write.enable_compression(threshold.0);
                                                client_read.enable_compression(threshold.0);
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                                debug!("Server -> Client: Packet ID: 0x{:02x}", packet.id);
                                client_write.write_packet(&packet).await?;
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
