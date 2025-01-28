use crate::network::connection::Connection;
use crate::network::packet::{Packet, PacketCodec};
use crate::protocol::minecraft::java::handshake::ServerBoundHandshake;
use crate::protocol::minecraft::java::login::clientbound_loginsuccess::{
    ClientBoundLoginSuccess, Property,
};
use crate::protocol::minecraft::java::login::{
    ClientBoundEncryptionRequest, ServerBoundEncryptionResponse,
};
use crate::protocol::types::{Boolean, Byte, ByteArray, ProtocolString, UnsignedShort, VarInt};
use crate::server::ServerResponse;
use crate::version::Version;
use crate::{EncryptionState, ProtocolRead};
use async_trait::async_trait;
use log::{debug, error, info};
use reqwest::Client;
use serde::Deserialize;
use std::io::{self};
use uuid::Uuid;

use super::{ClientProxyModeHandler, ProxyMessage, ProxyModeMessageType, ServerProxyModeHandler};
use crate::core::actors::client::MinecraftClient;
use crate::core::actors::server::{self, MinecraftServer};
use crate::core::event::MinecraftCommunication;
use crate::network::connection::PossibleReadValue;

pub struct ClientOnlyMode;

#[derive(Debug)]
pub enum ClientOnlyMessage {
    ClientReady(),
    ServerReady(),
    ClientLoginAknowledged(Packet),

    ServerThreshold(VarInt),
}

#[derive(Debug, Deserialize)]
struct MojangResponse {
    id: String,
    name: String,
    properties: Vec<Property>,
}

#[async_trait]
impl ClientProxyModeHandler<MinecraftCommunication<ClientOnlyMessage>> for ClientOnlyMode {
    async fn handle_internal_client(
        &self,
        message: MinecraftCommunication<ClientOnlyMessage>,
        actor: &mut MinecraftClient<MinecraftCommunication<ClientOnlyMessage>>,
    ) -> io::Result<()> {
        match message {
            MinecraftCommunication::Packet(data) => {
                actor.conn.write_packet(&data).await?;
            }
            MinecraftCommunication::Shutdown => {
                debug!("Shutting down client (Received Shutdown message)");
                actor.conn.close().await?;
            }
            _ => {
                info!("Unhandled message: {:?}", message);
            }
        }
        Ok(())
    }

    async fn handle_external_client(
        &self,
        data: PossibleReadValue,
        actor: &mut MinecraftClient<MinecraftCommunication<ClientOnlyMessage>>,
    ) -> io::Result<()> {
        match data {
            PossibleReadValue::Packet(data) => {
                let _ = actor
                    .server_sender
                    .send(MinecraftCommunication::Packet(data))
                    .await;
            }
            _ => {}
        }
        Ok(())
    }

    async fn initialize_client(
        &self,
        actor: &mut MinecraftClient<MinecraftCommunication<ClientOnlyMessage>>,
    ) -> io::Result<()> {
        let mut server_initialised = false;
        let mut threshold = VarInt(0);
        while !server_initialised {
            if let Some(msg) = actor.client_receiver.recv().await {
                match msg {
                    MinecraftCommunication::CustomData(ClientOnlyMessage::ServerReady()) => {
                        debug!("Server Ready, waiting for Server Threshold");
                        server_initialised = true;
                    }
                    MinecraftCommunication::CustomData(ClientOnlyMessage::ServerThreshold(th)) => {
                        threshold.0 = th.0;
                    }
                    _ => {}
                }
            }
        }
        debug!("Server Initialised, Threshold: {}", threshold.0);

        let mut encryption = EncryptionState::new();

        let mut request_packet = Packet::new(0x01);
        let enc_request = ClientBoundEncryptionRequest {
            server_id: ProtocolString("".to_string()),
            public_key: ByteArray(encryption.get_public_key_bytes()),
            verify_token: ByteArray(encryption.get_verify_token()),
            requires_authentication: Boolean(true),
        };

        request_packet.encode(&enc_request)?;
        debug!("P -> C: Sending encryption request");
        debug!("Public key length: {}", enc_request.public_key.0.len());
        debug!("Verify token length: {}", enc_request.verify_token.0.len());

        let mut compression_packet: Packet = Packet::new(0x03);
        compression_packet.encode(&threshold)?;
        actor.conn.write_packet(&compression_packet).await?;

        // Configure compression for all connections
        actor.conn.enable_compression(threshold.0);

        // 2. Wait and process client response
        debug!("Waiting for client encryption response");
        debug!("Sending packet: {:?}", request_packet);
        actor.conn.write_packet(&request_packet).await?;

        let response = actor.conn.read_packet().await?;
        if response.id != 0x01 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Expected encryption response (0x01), got 0x{:02x}",
                    response.id
                ),
            ));
        }

        let enc_response = ServerBoundEncryptionResponse::try_from(&response)?;
        debug!("Received encryption response from client");

        // 3. Decrypt and verify shared secret and token
        let shared_secret = encryption.decrypt_shared_secret(&enc_response.shared_secret.0)?;
        debug!(
            "Decrypted shared secret length: {}, Raw Data: {:?}",
            shared_secret.len(),
            shared_secret
        );

        // Verify shared secret length
        if shared_secret.len() != 16 {
            error!("Invalid shared secret length: {}", shared_secret.len());
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid shared secret length",
            ));
        }

        // Verify token before enabling encryption
        let tokent_similar = encryption.verify_encrypted_token(&enc_response.verify_token.0);
        if !tokent_similar {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Verify token mismatch",
            ));
        }

        // Enable encryption immediately after token verification
        debug!("Setting up encryption with shared secret");
        encryption.set_shared_secret(shared_secret.clone());
        actor.conn.enable_encryption(&encryption);

        // 5. Verify authentication with Mojang
        let server_hash = encryption.compute_server_id_hash("");
        debug!("Generated server hash: {}", server_hash);

        let url = format!(
            "https://sessionserver.mojang.com/session/minecraft/hasJoined?serverId={}&username={}",
            server_hash, actor.username
        );

        debug!("Verifying with Mojang API (URL: {})", url);
        let client = Client::new();

        let auth_response = client.get(&url).send().await.map_err(|e| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("Mojang API request failed: {}", e),
            )
        })?;

        let status = auth_response.status();
        debug!("Mojang API response status: {}", status);

        let response = match status.is_success() && status.as_u16() == 200 {
            true => {
                debug!("Authentication successful (200 OK)");
                let response_body = auth_response.json::<MojangResponse>().await.map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Failed to parse Mojang response: {}", e),
                    )
                });
                response_body?
            }
            false => {
                let error_text = auth_response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                error!("Authentication failed: {} - {}", status, error_text);
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("Authentication failed: {} - {}", status, error_text),
                ));
            }
        };

        debug!(
            "Successfully authenticated user: {} ({})",
            actor.username, response.id
        );

        if response.name != actor.username {
            error!("Username mismatch: {} != {}", response.name, actor.username);
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Username mismatch",
            ));
        }

        // 6. Send Login Success with UUID in correct format (with hyphens)
        let formatted_uuid = format!(
            "{}-{}-{}-{}-{}",
            &response.id[..8],
            &response.id[8..12],
            &response.id[12..16],
            &response.id[16..20],
            &response.id[20..]
        );

        let uuid = Uuid::parse_str(&formatted_uuid).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("Invalid UUID: {}", e))
        })?;

        let login_success = ClientBoundLoginSuccess {
            uuid,
            username: ProtocolString(actor.username.to_string()),
            properties: response.properties,
        };

        let mut success_packet = Packet::from(&login_success);
        let filtered_properties: Vec<Property> = login_success
            .properties
            .into_iter()
            .filter(|p| p.name.0 == "textures") // Keep only essential textures
            .collect();

        for prop in filtered_properties {
            success_packet.encode(&prop.name)?;
            // Limit value length if necessary
            success_packet.encode(&prop.value)?;
            success_packet.encode(&Boolean(prop.signature.is_some()))?;
            if let Some(sig) = prop.signature {
                success_packet.encode(&sig)?;
            }
        }

        actor.conn.write_packet(&success_packet).await?;

        let login_aknowledged = actor.conn.read_packet().await?;
        if login_aknowledged.id != 0x03 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Expected login aknowledged (0x03), got 0x{:02x}",
                    login_aknowledged.id
                ),
            ));
        }

        actor
            .server_sender
            .send(MinecraftCommunication::CustomData(
                ClientOnlyMessage::ClientLoginAknowledged(login_aknowledged),
            ))
            .await
            .unwrap();

        // Prepare Login Success packet but do not send it now
        info!(
            "Succes Packet prepared for authenticated user: {}",
            actor.username
        );

        debug!("Initializing client offline proxy mode");
        Ok(())
    }
}

#[async_trait]
impl ServerProxyModeHandler<MinecraftCommunication<ClientOnlyMessage>> for ClientOnlyMode {
    async fn handle_external_server(
        &self,
        data: PossibleReadValue,
        actor: &mut MinecraftServer<MinecraftCommunication<ClientOnlyMessage>>,
    ) -> io::Result<()> {
        match data {
            PossibleReadValue::Packet(data) => {
                let _ = actor
                    .client_sender
                    .send(MinecraftCommunication::Packet(data))
                    .await;
            }
            _ => {}
        }

        Ok(())
    }

    async fn handle_internal_server(
        &self,
        message: MinecraftCommunication<ClientOnlyMessage>,
        actor: &mut MinecraftServer<MinecraftCommunication<ClientOnlyMessage>>,
    ) -> io::Result<()> {
        match message {
            MinecraftCommunication::Packet(data) => {
                debug!("Received packet from server: {:?}", data);
                actor
                    .server_request
                    .as_mut()
                    .unwrap()
                    .server_conn
                    .as_mut()
                    .unwrap()
                    .write_packet(&data)
                    .await?;
            }
            MinecraftCommunication::Shutdown => {
                debug!("Shutting down server (Received Shutdown message)");
                actor
                    .server_request
                    .as_mut()
                    .unwrap()
                    .server_conn
                    .as_mut()
                    .unwrap()
                    .close()
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn initialize_server(
        &self,
        actor: &mut MinecraftServer<MinecraftCommunication<ClientOnlyMessage>>,
    ) -> io::Result<()> {
        debug!("Initializing server offline proxy mode");
        let mut server_initialised = false;

        if let Some(request) = actor.server_request.as_mut() {
            if let Some(conn) = request.server_conn.as_mut() {
                let client_handshake = &request.read_packets[0];
                let server_addr = conn.peer_addr().await?;
                let login_start = &request.read_packets[1];

                let server_handshake = prepare_server_handshake(client_handshake, &server_addr)?;
                conn.write_packet(&server_handshake).await?;
                conn.write_packet(login_start).await?;

                while !server_initialised {
                    match conn.read_packet().await? {
                        packet if packet.id == 0x03 => {
                            // Set Compression
                            let threshold = packet.decode::<VarInt>()?;
                            if threshold.0 >= 0 {
                                debug!(
                                    "Received compression config from server, threshold: {}",
                                    threshold.0
                                );

                                conn.enable_compression(threshold.0);
                                match actor
                                    .client_sender
                                    .send(MinecraftCommunication::CustomData(
                                        ClientOnlyMessage::ServerThreshold(threshold),
                                    ))
                                    .await
                                {
                                    Ok(_) => {}
                                    Err(e) => {
                                        error!(
                                            "Failed to send ServerThreshold message to client: {}",
                                            e
                                        );
                                    }
                                }
                            }
                        }
                        packet if packet.id == 0x02 => {
                            // Ignore server's Login Success, we'll use ours
                            debug!("Received Login Success from server, Server Initialised");
                            server_initialised = true;
                        }
                        packet => {
                            debug!("Received packet {:?} from server", packet);
                        }
                    }
                }

                match actor
                    .client_sender
                    .send(MinecraftCommunication::CustomData(
                        ClientOnlyMessage::ServerReady(),
                    ))
                    .await
                {
                    Ok(_) => {}
                    Err(e) => {
                        error!("Failed to send ServerReady message to client: {}", e);
                    }
                };

                if let Some(msg) = actor.server_receiver.recv().await {
                    match msg {
                        MinecraftCommunication::CustomData(
                            ClientOnlyMessage::ClientLoginAknowledged(packet),
                        ) => {
                            conn.write_packet(&packet).await?;
                        }
                        _ => {
                            error!("Unexpected message waited Aknowledge got : {:?}", msg);
                        }
                    }
                }
            }
        }

        if let Some(server_request) = &mut actor.server_request {
            if let Some(server_conn) = &mut server_request.server_conn {
                for packet in server_request.read_packets.iter() {
                    server_conn.write_packet(packet).await?;
                }
            } else {
                error!("Server connection is None");
            }
        } else {
            error!("Server request is None");
        }
        Ok(())
    }
}
impl ProxyMessage for ClientOnlyMessage {}

impl ProxyModeMessageType for ClientOnlyMode {
    type Message = ClientOnlyMessage;
}

fn prepare_server_handshake(
    client_handshake: &Packet,
    server_addr: &std::net::SocketAddr,
) -> io::Result<Packet> {
    let mut cursor = std::io::Cursor::new(&client_handshake.data);
    let (protocol_version, _) = VarInt::read_from(&mut cursor)?;

    let server_handshale = ServerBoundHandshake {
        protocol_version,
        server_address: ProtocolString(server_addr.ip().to_string()),
        server_port: UnsignedShort(server_addr.port()),
        next_state: Byte(2),
    };

    let handshake = Packet::try_from(&server_handshale).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to create server handshake packet: {}", e),
        )
    })?;
    Ok(handshake)
}
