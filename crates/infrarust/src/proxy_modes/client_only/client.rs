use std::{
    error::Error,
    io::{self},
};

use async_trait::async_trait;
use infrarust_config::LogType;
use infrarust_protocol::{
    minecraft::java::login::{
        ClientBoundEncryptionRequest, ServerBoundEncryptionResponse,
        clientbound_loginsuccess::{ClientBoundLoginSuccess, Property},
    },
    types::{Boolean, ByteArray, ProtocolString, VarInt},
};
use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, error, info};
use uuid::Uuid;

use crate::{
    EncryptionState,
    core::{actors::client::MinecraftClient, event::MinecraftCommunication},
    network::{
        connection::PossibleReadValue,
        packet::{Packet, PacketCodec},
    },
    proxy_modes::{ClientProxyModeHandler, client_only::ClientOnlyMessage},
};

use super::ClientOnlyMode;

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
                debug!(
                    log_type = LogType::ProxyMode.as_str(),
                    "Shutting down client (Received Shutdown message)"
                );
                actor.conn.close().await?;
            }
            _ => {
                info!(
                    log_type = LogType::ProxyMode.as_str(),
                    "Unhandled message: {:?}", message
                );
            }
        }
        Ok(())
    }

    async fn handle_external_client(
        &self,
        data: PossibleReadValue,
        actor: &mut MinecraftClient<MinecraftCommunication<ClientOnlyMessage>>,
    ) -> io::Result<()> {
        if let PossibleReadValue::Packet(data) = data {
            let _ = actor
                .server_sender
                .send(MinecraftCommunication::Packet(data))
                .await;
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
                        debug!(
                            log_type = LogType::ProxyMode.as_str(),
                            "Server Ready, waiting for Server Threshold"
                        );
                        server_initialised = true;
                    }
                    MinecraftCommunication::CustomData(ClientOnlyMessage::ServerThreshold(th)) => {
                        threshold.0 = th.0;
                    }
                    _ => {}
                }
            }
        }
        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Server Initialised, Threshold: {}", threshold.0
        );

        let mut encryption = EncryptionState::new();

        let mut request_packet = Packet::new(0x01);
        let enc_request = ClientBoundEncryptionRequest {
            server_id: ProtocolString("".to_string()),
            public_key: ByteArray(encryption.get_public_key_bytes()),
            verify_token: ByteArray(encryption.get_verify_token()),
            requires_authentication: Boolean(true),
        };

        let _ = request_packet.encode(&enc_request);
        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "P -> C: Sending encryption request"
        );
        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Public key length: {}",
            enc_request.public_key.0.len()
        );
        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Verify token length: {}",
            enc_request.verify_token.0.len()
        );

        let mut compression_packet: Packet = Packet::new(0x03);
        let _ = compression_packet.encode(&threshold);
        actor.conn.write_packet(&compression_packet).await?;

        // Configure compression for all connections
        actor.conn.enable_compression(threshold.0);

        // 2. Wait and process client response
        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Waiting for client encryption response"
        );
        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Sending packet: {:?}", request_packet
        );
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
        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Received encryption response from client"
        );

        // 3. Decrypt and verify shared secret and token
        let shared_secret = encryption.decrypt_shared_secret(&enc_response.shared_secret.0)?;
        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Decrypted shared secret length: {}, Raw Data: {:?}",
            shared_secret.len(),
            shared_secret
        );

        // Verify shared secret length
        if shared_secret.len() != 16 {
            error!(
                log_type = LogType::ProxyMode.as_str(),
                "Invalid shared secret length: {}",
                shared_secret.len()
            );
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
        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Setting up encryption with shared secret"
        );
        encryption.set_shared_secret(shared_secret.clone());
        actor.conn.enable_encryption(&encryption);

        // 5. Verify authentication with Mojang
        let server_hash = encryption.compute_server_id_hash("");
        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Generated server hash: {}", server_hash
        );

        let url = format!(
            "https://sessionserver.mojang.com/session/minecraft/hasJoined?serverId={}&username={}",
            server_hash, actor.username
        );

        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Verifying with Mojang API (URL: {})", url
        );
        let client = Client::new();

        let auth_response = client.get(&url).send().await.map_err(|e| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("Mojang API request failed: {:?}", e.source()),
            )
        })?;

        let status = auth_response.status();
        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Mojang API response status: {}", status
        );

        let response = match status.is_success() && status.as_u16() == 200 {
            true => {
                debug!(
                    log_type = LogType::ProxyMode.as_str(),
                    "Authentication successful (200 OK)"
                );
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
                error!(
                    log_type = LogType::ProxyMode.as_str(),
                    "Authentication failed: {} - {}", status, error_text
                );
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("Authentication failed: {} - {}", status, error_text),
                ));
            }
        };

        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Successfully authenticated user: {} ({})", actor.username, response.id
        );

        if response.name != actor.username {
            error!(
                log_type = LogType::ProxyMode.as_str(),
                "Username mismatch: {} != {}", response.name, actor.username
            );
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
            let _ = success_packet.encode(&prop.name);
            // Limit value length if necessary
            let _ = success_packet.encode(&prop.value);
            let _ = success_packet.encode(&Boolean(prop.signature.is_some()));
            if let Some(sig) = prop.signature {
                let _ = success_packet.encode(&sig);
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
            log_type = LogType::ProxyMode.as_str(),
            "Succes Packet prepared for authenticated user: {}", actor.username
        );

        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Initializing client offline proxy mode"
        );
        Ok(())
    }
}
