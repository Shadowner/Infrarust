use super::ProxyModeHandler;
use crate::network::connection::Connection;
use crate::network::packet::{Packet, PacketCodec, PacketReader, PacketWriter};
use crate::protocol::minecraft::java::handshake::ServerBoundHandshake;
use crate::protocol::minecraft::java::login::clientbound_loginsuccess::{
    ClientBoundLoginSuccess, Property,
};
use crate::protocol::minecraft::java::login::{
    clientbound_encryptionrequest::ClientBoundEncryptionRequest,
    serverbound_encryptionresponse::ServerBoundEncryptionResponse,
    serverbound_loginstart::ServerBoundLoginStart,
};
use crate::protocol::types::{Boolean, Byte, ByteArray, ProtocolString, UnsignedShort, VarInt};
use crate::security::encryption::EncryptionState;
use crate::server::ServerResponse;
use crate::version::Version;
use crate::ProtocolRead;
use async_trait::async_trait;
use core::panic;
use log::{debug, error, info};
use reqwest::Client;
use serde::Deserialize;
use std::io::{self};
use tokio::io::{BufReader, BufWriter};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct MojangResponse {
    id: String,
    name: String,
    properties: Vec<Property>,
}

pub struct ClientOnlyMode;

#[async_trait]
impl ProxyModeHandler for ClientOnlyMode {
    async fn handle(
        &self,
        client: Connection,
        response: ServerResponse,
        proxy_protocol: Version,
    ) -> io::Result<()> {
        let mut client_encryption = EncryptionState::new();
        let server = response.server_conn.unwrap();
        let server_addr = server.peer_addr().await?;

        debug!("=== Starting client-only mode ===");

        let (mut client_read, mut client_write) = client.into_split();
        let (mut server_read, mut server_write) = server.into_split();

        client_read.set_protocol_version(proxy_protocol);
        server_read.set_protocol_version(proxy_protocol);

        let client_handshake = &response.read_packets[0];
        let login_start = &response.read_packets[1];
        let username = ServerBoundLoginStart::try_from(login_start)?.name.0;

        //TODO: Check why clientOnly doesn't work with Forge clients

        // 1. Send handshake and login start to server
        debug!("Connecting to backend server: {}", server_addr);
        let server_handshake = prepare_server_handshake(client_handshake, &server_addr)?;
        server_write.write_packet(&server_handshake).await?;
        server_write.write_packet(login_start).await?;
        let mut server_initialised = false;

        // 3. Wait and configure compression if needed
        while !server_initialised {
            match server_read.read_packet().await? {
                packet if packet.id == 0x03 => {
                    // Set Compression
                    let threshold = packet.decode::<VarInt>()?;
                    if threshold.0 >= 0 {
                        debug!(
                            "Received compression config from server, threshold: {}",
                            threshold.0
                        );

                        server_write.enable_compression(threshold.0);
                        server_read.enable_compression(threshold.0);
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

        debug!("Starting client authentication sequence");
        let success = authenticate_client(
            &mut client_encryption,
            &mut client_write,
            &mut client_read,
            &username,
        )
        .await?;

        // 4. Send Login Success to client
        debug!(
            "Client Write: Compression {}, Encryption {}",
            client_write.is_compression_enabled(),
            client_write.is_encryption_enabled()
        );

        debug!("Sending Login Success to client, packet {:?} ", success);
        debug!(
            "Client Encryptiuon {:?}",
            client_write.is_encryption_enabled()
        );

        let mut success_packet = Packet::from(&success);

        // Filter properties if necessary
        let filtered_properties: Vec<Property> = success
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

        debug!(
            "Login Success packet prepared, raw size: {}",
            success_packet.data.len()
        );

        // Compression and encryption will be applied by write_packet
        client_write.write_packet(&success_packet).await?;

        // 5. Wait for Login Acknowledged from client
        debug!("Waiting for Login Acknowledged from client");
        loop {
            let packet = client_read.read_packet().await?;
            debug!("Received packet 0x{:02x} from client", packet.id);
            if packet.id == 0x03 {
                // Login Acknowledged
                debug!("Received Login Acknowledged from client");
                server_write.write_packet(&packet).await?;
                break;
            }
        }

        debug!("=== Login sequence completed, entering play phase ===");
        handle_play_phase(
            &mut client_write,
            &mut client_read,
            &mut server_write,
            &mut server_read,
        )
        .await
    }
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

async fn authenticate_client(
    encryption: &mut EncryptionState,
    client_write: &mut PacketWriter<BufWriter<OwnedWriteHalf>>,
    client_read: &mut PacketReader<BufReader<OwnedReadHalf>>,
    username: &str,
) -> io::Result<ClientBoundLoginSuccess> {
    // 1. Send encryption request to client with correct data
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

    let mut compression_packet = Packet::new(0x03);
    compression_packet.encode(&VarInt(256))?; // Standard threshold
    client_write.write_packet(&compression_packet).await?;

    // Configure compression for all connections
    client_write.enable_compression(256);
    client_read.enable_compression(256);

    // 2. Wait and process client response
    debug!("Waiting for client encryption response");
    client_write.write_packet(&request_packet).await?;
    let response = client_read.read_packet().await?;
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

    if let Some((enc, dec)) = encryption.create_cipher() {
        debug!("Enabling client encryption - Creating cipher pair with shared secret");
        client_write.enable_encryption(enc);
        client_read.enable_encryption(dec);
        debug!("Client encryption enabled successfully");
    } else {
        error!("Failed to create cipher pair");
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Failed to create cipher",
        ));
    }

    // 5. Verify authentication with Mojang
    let server_hash = encryption.compute_server_id_hash("");
    debug!("Generated server hash: {}", server_hash);

    let url = format!(
        "https://sessionserver.mojang.com/session/minecraft/hasJoined?serverId={}&username={}",
        server_hash, username
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
        username, response.id
    );

    if response.name != username {
        error!("Username mismatch: {} != {}", response.name, username);
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

    let uuid = Uuid::parse_str(&formatted_uuid)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Invalid UUID: {}", e)))?;

    // Prepare Login Success packet but do not send it now
    info!(
        "Succes Packet prepared for authenticated user: {}",
        username
    );
    Ok(ClientBoundLoginSuccess {
        uuid,
        username: ProtocolString(username.to_string()),
        properties: response.properties,
    })
}
async fn handle_play_phase(
    client_write: &mut PacketWriter<BufWriter<OwnedWriteHalf>>,
    client_read: &mut PacketReader<BufReader<OwnedReadHalf>>,
    server_write: &mut PacketWriter<BufWriter<OwnedWriteHalf>>,
    server_read: &mut PacketReader<BufReader<OwnedReadHalf>>,
) -> io::Result<()> {
    debug!("=== Starting configuration phase ===");

    // Wait and transfer three configuration packets
    let mut config_packets_received = 0;
    while config_packets_received < 3 {
        match server_read.read_packet().await {
            Ok(packet) => {
                debug!(
                    "Relaying configuration packet 0x{:02x} to client",
                    packet.id
                );
                client_write.write_packet(&packet).await?;
                config_packets_received += 1;
            }
            Err(e) => {
                error!("Error reading configuration packet: {}", e);
                return Err(e.into());
            }
        }
    }

    // Wait for finish_configuration from client
    match client_read.read_packet().await {
        Ok(packet) => {
            debug!("Relaying finish_configuration from client to server");
            server_write.write_packet(&packet).await?;
        }
        Err(e) => {
            error!("Error reading finish_configuration: {}", e);
            return Err(e.into());
        }
    }

    debug!("=== Configuration phase completed, entering play phase ===");

    // Rest of play phase code
    loop {
        tokio::select! {
            client_result = client_read.read_packet() => {
                match client_result {
                    Ok(packet) => {
                        debug!("C -> P -> S: Relaying packet 0x{:02x}", packet.id);
                        server_write.write_packet(&packet).await?;
                    }
                    Err(e) => {
                        error!("Client read error: {}", e);
                        break;
                    }
                }
            }
            server_result = server_read.read_packet() => {
                match server_result {
                    Ok(packet) => {
                        debug!("S -> P -> C: Relaying packet 0x{:02x}", packet.id);
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

    debug!("Play phase ended");
    Ok(())
}
