//! Mojang authentication for `ClientOnly` proxy mode.
//!
//! Handles RSA key exchange, SHA-1 server hash computation,
//! and session verification against `sessionserver.mojang.com`.

use num_bigint::BigInt;
use rand::RngCore;
use rsa::pkcs8::EncodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};
use sha1::{Digest, Sha1};

use infrarust_protocol::packets::login::{CEncryptionRequest, SEncryptionResponse};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction};

use crate::auth::game_profile::GameProfile;
use crate::error::CoreError;
use crate::session::client_bridge::ClientBridge;

const DEFAULT_SESSION_URL: &str = "https://sessionserver.mojang.com/session/minecraft/hasJoined";

/// Handles Mojang authentication for the `ClientOnly` proxy mode.
///
/// A single instance is shared across all connections (via `Arc`).
/// The RSA key and HTTP client are reused for every authentication.
pub struct MojangAuth {
    rsa_key: RsaPrivateKey,
    public_key_der: Vec<u8>,
    http_client: reqwest::Client,
    session_url: String,
}

impl MojangAuth {
    /// Creates a new `MojangAuth` with a fresh RSA-1024 key.
    ///
    /// # Errors
    /// Returns `CoreError::Auth` if RSA key generation or DER encoding fails.
    pub fn new() -> Result<Self, CoreError> {
        Self::build(DEFAULT_SESSION_URL.to_string())
    }

    /// Creates a `MojangAuth` with a custom session server URL (for testing).
    ///
    /// # Errors
    /// Returns `CoreError::Auth` if RSA key generation or DER encoding fails.
    pub fn with_session_url(session_url: String) -> Result<Self, CoreError> {
        Self::build(session_url)
    }

    fn build(session_url: String) -> Result<Self, CoreError> {
        let mut rng = rand::rngs::OsRng;
        let rsa_key = RsaPrivateKey::new(&mut rng, 1024)
            .map_err(|e| CoreError::Auth(format!("RSA key generation failed: {e}")))?;

        let public_key_der = rsa_key
            .to_public_key()
            .to_public_key_der()
            .map_err(|e| CoreError::Auth(format!("DER encoding failed: {e}")))?
            .to_vec();

        let http_client = reqwest::Client::new();

        Ok(Self {
            rsa_key,
            public_key_der,
            http_client,
            session_url,
        })
    }

    /// Runs the full Mojang authentication flow.
    ///
    /// 1. Sends `EncryptionRequest` to client
    /// 2. Reads `EncryptionResponse`
    /// 3. Decrypts and verifies shared secret + verify token
    /// 4. Computes server hash and calls Mojang session server
    /// 5. Enables encryption on the client bridge
    /// 6. Returns the authenticated `GameProfile`
    ///
    /// # Errors
    /// Returns `CoreError::Auth` on RSA decrypt failure, token mismatch,
    /// or session server verification failure.
    ///
    /// # Panics
    /// Panics if the shared secret is validated as 16 bytes but the
    /// `try_into` conversion still fails (should be unreachable).
    pub async fn authenticate(
        &self,
        client: &mut ClientBridge,
        username: &str,
        registry: &PacketRegistry,
    ) -> Result<GameProfile, CoreError> {
        // Generate random verify token
        let mut verify_token = [0u8; 4];
        rand::rngs::OsRng.fill_bytes(&mut verify_token);

        // Send EncryptionRequest
        let version = client.protocol_version;
        let enc_request = CEncryptionRequest {
            server_id: String::new(),
            public_key: self.public_key_der.clone(),
            verify_token: verify_token.to_vec(),
            should_authenticate: true,
        };
        client.send_packet(&enc_request, registry).await?;

        // Read EncryptionResponse
        let frame = client
            .read_frame()
            .await?
            .ok_or(CoreError::ConnectionClosed)?;

        let decoded = registry.decode_frame(
            &frame,
            ConnectionState::Login,
            Direction::Serverbound,
            version,
        )?;

        let enc_response = match decoded {
            DecodedPacket::Typed { packet, .. } => packet
                .as_any()
                .downcast_ref::<SEncryptionResponse>()
                .ok_or_else(|| {
                    CoreError::Auth(format!(
                        "expected EncryptionResponse, got {}",
                        packet.packet_name()
                    ))
                })?
                .clone(),
            DecodedPacket::Opaque { id, .. } => {
                return Err(CoreError::Auth(format!(
                    "expected EncryptionResponse, got opaque packet 0x{id:02x}"
                )));
            }
        };

        // Decrypt shared secret
        let shared_secret = self
            .rsa_key
            .decrypt(Pkcs1v15Encrypt, &enc_response.shared_secret)
            .map_err(|e| CoreError::Auth(format!("shared secret decrypt failed: {e}")))?;

        if shared_secret.len() != 16 {
            return Err(CoreError::Auth(format!(
                "shared secret must be 16 bytes, got {}",
                shared_secret.len()
            )));
        }

        // Decrypt verify token
        let decrypted_token = self
            .rsa_key
            .decrypt(Pkcs1v15Encrypt, &enc_response.verify_token)
            .map_err(|e| CoreError::Auth(format!("verify token decrypt failed: {e}")))?;

        // Verify token matches
        if decrypted_token != verify_token {
            return Err(CoreError::Auth("verify token mismatch".to_string()));
        }

        // Compute server hash
        let server_hash = minecraft_server_hash("", &shared_secret, &self.public_key_der);

        // Call Mojang session server
        let profile = self.verify_session(username, &server_hash).await?;

        // Enable encryption on client bridge
        #[allow(clippy::expect_used)] // Length already validated above
        let key: [u8; 16] = shared_secret
            .try_into()
            .expect("shared secret length already validated as 16");
        client.enable_encryption(&key);

        Ok(profile)
    }

    /// Verifies the player session against the Mojang session server.
    async fn verify_session(
        &self,
        username: &str,
        server_hash: &str,
    ) -> Result<GameProfile, CoreError> {
        let response = self
            .http_client
            .get(&self.session_url)
            .query(&[("username", username), ("serverId", server_hash)])
            .send()
            .await
            .map_err(|e| CoreError::Auth(format!("session server request failed: {e}")))?;

        if !response.status().is_success() {
            return Err(CoreError::Auth(format!(
                "session server returned {}",
                response.status()
            )));
        }

        let profile: GameProfile = response
            .json()
            .await
            .map_err(|e| CoreError::Auth(format!("failed to parse game profile: {e}")))?;

        Ok(profile)
    }
}

/// Computes the Minecraft server hash (non-standard SHA-1).
///
/// The result is a signed `BigInt` in hexadecimal — negative values
/// are prefixed with `-`. This matches the Minecraft protocol spec
/// documented on `https://minecraft.wiki/w/Java_Edition_protocol/Encryption`.
///
///
/// ```
/// use infrarust_core::auth::mojang::minecraft_server_hash;
///
/// assert_eq!(
///     minecraft_server_hash("Notch", &[], &[]),
///     "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48"
/// );
/// assert_eq!(
///     minecraft_server_hash("jeb_", &[], &[]),
///     "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1"
/// );
/// ```
pub fn minecraft_server_hash(
    server_id: &str,
    shared_secret: &[u8],
    public_key_der: &[u8],
) -> String {
    let mut hasher = Sha1::new();
    hasher.update(server_id.as_bytes());
    hasher.update(shared_secret);
    hasher.update(public_key_der);
    let digest = hasher.finalize();
    BigInt::from_signed_bytes_be(&digest).to_str_radix(16)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_server_hash_notch() {
        assert_eq!(
            minecraft_server_hash("Notch", &[], &[]),
            "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48"
        );
    }

    #[test]
    fn test_server_hash_jeb() {
        assert_eq!(
            minecraft_server_hash("jeb_", &[], &[]),
            "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1"
        );
    }

    #[test]
    fn test_server_hash_simon() {
        assert_eq!(
            minecraft_server_hash("simon", &[], &[]),
            "88e16a1019277b15d58faf0541e11910eb756f6"
        );
    }

    #[test]
    fn test_mojang_auth_creation() {
        let auth = MojangAuth::new();
        assert!(auth.is_ok());
        let auth = auth.unwrap();
        // Public key should be non-empty DER
        assert!(!auth.public_key_der.is_empty());
    }
}
