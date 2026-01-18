use aes::cipher::KeyIvInit;
use infrarust_config::LogType;
use infrarust_protocol::minecraft::java::sha1::generate_server_hash;
use rand::RngCore;
use rsa::{
    Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey,
    pkcs1::DecodeRsaPublicKey,
    pkcs8::{DecodePublicKey, EncodePublicKey},
    traits::PublicKeyParts,
};
use tracing::{debug, error};

pub type Aes128Cfb8Enc = cfb8::Encryptor<aes::Aes128>;
pub type Aes128Cfb8Dec = cfb8::Decryptor<aes::Aes128>;

use aes::cipher::{
    BlockBackend, BlockClosure, BlockSizeUser, generic_array::GenericArray, inout::InOut,
};
use generic_array::typenum::U1;

use crate::RsaError;

pub struct Cfb8Closure<'a> {
    pub data: &'a mut [u8],
}

impl BlockSizeUser for Cfb8Closure<'_> {
    type BlockSize = U1;
}

impl BlockClosure for Cfb8Closure<'_> {
    fn call<B: BlockBackend<BlockSize = Self::BlockSize>>(self, backend: &mut B) {
        for byte in self.data.iter_mut() {
            let input = GenericArray::<u8, U1>::from([*byte]);
            let mut output = GenericArray::<u8, U1>::default();
            let block = InOut::from((&input, &mut output));
            backend.proc_block(block);
            *byte = output[0];
        }
    }
}

pub struct EncryptionState {
    shared_secret: Vec<u8>,
    verify_token: Vec<u8>,
    private_key: RsaPrivateKey,
    public_key: RsaPublicKey,
    public_key_der: Vec<u8>,
    server_public_key: Option<RsaPublicKey>,
}

impl Default for EncryptionState {
    fn default() -> Self {
        Self::try_new().expect("Failed to initialize encryption state - this is a critical error")
    }
}

impl EncryptionState {
    pub fn try_new() -> Result<Self, RsaError> {
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 1024)
            .map_err(|e| RsaError::KeyGenerationError(e.to_string()))?;
        let public_key = RsaPublicKey::from(&private_key);

        // Generate and store the public key in DER format
        let public_key_der = public_key
            .to_public_key_der()
            .map_err(|e| RsaError::KeyEncodingError(e.to_string()))?
            .as_ref()
            .to_vec();

        let mut verify_token = vec![0u8; 4];
        rng.fill_bytes(&mut verify_token);

        Ok(Self {
            shared_secret: Vec::new(),
            verify_token,
            private_key,
            public_key,
            public_key_der,
            server_public_key: None,
        })
    }

    /// Creates a new encryption state.
    /// Panics if key generation fails (unlikely in normal operation).
    pub fn new() -> Self {
        Self::try_new().expect("Failed to initialize encryption state")
    }

    /// Creates a new encryption state with server data.
    /// Returns an error if the server public key cannot be parsed or key generation fails.
    pub fn try_new_with_server_data(
        server_id: String,
        public_key_bytes: Vec<u8>,
        verify_token: Vec<u8>,
    ) -> Result<Self, RsaError> {
        debug!(
            log_type = LogType::Authentication.as_str(),
            "Creating new encryption state with server data"
        );
        debug!(
            log_type = LogType::Authentication.as_str(),
            "Server ID: {}", server_id
        );
        debug!(
            log_type = LogType::Authentication.as_str(),
            "Public key length: {}",
            public_key_bytes.len()
        );
        debug!(
            log_type = LogType::Authentication.as_str(),
            "Verify token length: {}",
            verify_token.len()
        );

        // Parse the server's public key
        let server_public_key = RsaPublicKey::from_public_key_der(&public_key_bytes)
            .or_else(|_| RsaPublicKey::from_pkcs1_der(&public_key_bytes))
            .map_err(|e| RsaError::KeyEncodingError(format!("Failed to parse server public key: {}", e)))?;

        let private_key = RsaPrivateKey::new(&mut rand::thread_rng(), 1024)
            .map_err(|e| RsaError::KeyGenerationError(e.to_string()))?;
        let public_key = RsaPublicKey::from(&private_key);

        Ok(Self {
            shared_secret: Vec::new(),
            verify_token,
            private_key,
            public_key,
            server_public_key: Some(server_public_key),
            public_key_der: public_key_bytes,
        })
    }

    /// Creates a new encryption state with server data.
    /// Panics if the server public key cannot be parsed or key generation fails.
    pub fn new_with_server_data(
        server_id: String,
        public_key_bytes: Vec<u8>,
        verify_token: Vec<u8>,
    ) -> Self {
        Self::try_new_with_server_data(server_id, public_key_bytes, verify_token)
            .expect("Failed to initialize encryption state with server data")
    }

    pub fn verify_token_matches(&self, verify_token: &[u8]) -> bool {
        if self.server_public_key.is_some() {
            // En mode proxy, on ne vérifie pas le token car on ne peut pas le déchiffrer
            true
        } else {
            // En mode standard, comparer les tokens
            self.verify_token == verify_token
        }
    }

    pub fn verify_encrypted_token(&self, encrypted_token: &[u8]) -> bool {
        if self.server_public_key.is_some() {
            // En mode proxy, on ne peut pas vérifier le token
            true
        } else {
            // En mode standard, déchiffrer le token et comparer
            let decrypted = self
                .private_key
                .decrypt(Pkcs1v15Encrypt, encrypted_token)
                .map_err(RsaError::from)
                .ok();
            debug!(
                log_type = LogType::Authentication.as_str(),
                "Decrypted verify token: {:?} boolean: {:?}",
                decrypted,
                decrypted.clone().unwrap_or_default() == self.verify_token
            );
            decrypted.unwrap_or_default() == self.verify_token
        }
    }

    // Renommer l'ancienne méthode
    pub fn process_encrypted_secret(
        &mut self,
        encrypted_shared_secret: &[u8],
    ) -> Result<Vec<u8>, RsaError> {
        if let Some(ref server_key) = self.server_public_key {
            // En mode proxy, on déchiffre d'abord avec la clé privée du client
            let decrypted_secret = self
                .private_key
                .decrypt(Pkcs1v15Encrypt, encrypted_shared_secret)
                .map_err(RsaError::from)?;

            // Puis on stocke le secret déchiffré
            self.shared_secret = decrypted_secret.clone();

            // Et on le re-chiffre avec la clé publique du serveur
            let re_encrypted = server_key
                .encrypt(&mut rand::thread_rng(), Pkcs1v15Encrypt, &decrypted_secret)
                .map_err(RsaError::from)?;

            Ok(re_encrypted)
        } else {
            // Mode standard inchangé
            let shared_secret = self
                .private_key
                .decrypt(Pkcs1v15Encrypt, encrypted_shared_secret)
                .map_err(RsaError::from)?;

            self.shared_secret = shared_secret.clone();
            Ok(shared_secret)
        }
    }

    pub fn has_shared_secret(&self) -> bool {
        !self.shared_secret.is_empty()
    }

    pub fn encrypt_verify_token(&self, encrypted_verify_token: &[u8]) -> Result<Vec<u8>, RsaError> {
        if let Some(ref _server_key) = self.server_public_key {
            // En mode proxy, on forward directement le token chiffré
            Ok(encrypted_verify_token.to_vec())
        } else {
            // En mode standard, déchiffrer avec notre clé privée
            self.private_key
                .decrypt(Pkcs1v15Encrypt, encrypted_verify_token)
                .map_err(RsaError::from)
        }
    }

    pub fn get_public_key_der(&self) -> Vec<u8> {
        rsa_der::public_key_to_der(
            &self.private_key.n().to_bytes_be(),
            &self.private_key.e().to_bytes_be(),
        )
    }

    /// Get the public key bytes in X.509/DER format.
    /// Returns an error if encoding fails (unlikely after successful initialization).
    pub fn try_get_public_key_bytes(&self) -> Result<Vec<u8>, RsaError> {
        self.public_key
            .to_public_key_der()
            .map(|der| der.as_ref().to_vec())
            .map_err(|e| RsaError::KeyEncodingError(e.to_string()))
    }

    /// Get the public key bytes in X.509/DER format.
    /// Panics if encoding fails (unlikely after successful initialization).
    pub fn get_public_key_bytes(&self) -> Vec<u8> {
        self.try_get_public_key_bytes()
            .expect("Failed to encode public key to X.509")
    }

    pub fn create_cipher(&self) -> Option<(Aes128Cfb8Enc, Aes128Cfb8Dec)> {
        if !self.has_shared_secret() || self.shared_secret.len() != 16 {
            debug!(
                log_type = LogType::Authentication.as_str(),
                "Cannot create cipher: invalid shared secret"
            );
            return None;
        }

        // Utiliser les 16 octets comme clé et IV
        let key: &[u8; 16] = self.shared_secret
            .as_slice()
            .try_into()
            .expect("shared_secret must be exactly 16 bytes (validated above)");
        let iv: &[u8; 16] = key; // Même valeur pour l'IV

        let encrypt = Aes128Cfb8Enc::new(key.into(), iv.into());
        let decrypt = Aes128Cfb8Dec::new(key.into(), iv.into());

        debug!(
            log_type = LogType::Authentication.as_str(),
            "Created AES-128-CFB8 ciphers with 16-byte key/IV"
        );
        Some((encrypt, decrypt))
    }

    pub fn compute_server_id_hash(&self, server_id: &str) -> String {
        debug!(
            log_type = LogType::Authentication.as_str(),
            "Computing hash with public key length: {}",
            self.public_key_der.len()
        );
        generate_server_hash(server_id, &self.shared_secret, &self.public_key_der)
    }

    pub fn get_verify_token(&self) -> Vec<u8> {
        self.verify_token.clone()
    }

    // Méthode pour accéder au verify token pour l'API publique uniquement
    pub fn verify_token(&self) -> &[u8] {
        &self.verify_token
    }

    pub fn decrypt_shared_secret(&self, encrypted: &[u8]) -> Result<Vec<u8>, RsaError> {
        let decrypted = self
            .private_key
            .decrypt(Pkcs1v15Encrypt, encrypted)
            .map_err(RsaError::from)?;

        // Vérifier que le secret déchiffré fait 16 octets
        if decrypted.len() != 16 {
            error!(
                log_type = LogType::Authentication.as_str(),
                "Decrypted shared secret has invalid length: {}, expected 16",
                decrypted.len()
            );
            return Err(RsaError::InvalidKeyLength(decrypted.len()));
        }

        debug!(
            log_type = LogType::Authentication.as_str(),
            "Decrypted shared secret with correct length: 16 bytes"
        );
        Ok(decrypted)
    }

    pub fn encrypt_shared_secret(&self, secret: &[u8]) -> Result<Vec<u8>, RsaError> {
        if secret.len() != 16 {
            error!(
                log_type = LogType::Authentication.as_str(),
                "Invalid shared secret length: {}, expected 16",
                secret.len()
            );
            return Err(RsaError::InvalidKeyLength(secret.len()));
        }

        // Toujours utiliser PKCS1v1.5 avec un padding de 256 bits
        if let Some(ref server_key) = self.server_public_key {
            server_key
                .encrypt(&mut rand::thread_rng(), Pkcs1v15Encrypt, secret)
                .map_err(RsaError::from)
        } else {
            self.public_key
                .encrypt(&mut rand::thread_rng(), Pkcs1v15Encrypt, secret)
                .map_err(RsaError::from)
        }
    }

    pub fn set_shared_secret(&mut self, secret: Vec<u8>) {
        if secret.len() != 16 {
            error!(
                log_type = LogType::Authentication.as_str(),
                "Invalid shared secret length: {}, expected 16",
                secret.len()
            );
            return;
        }
        debug!(
            log_type = LogType::Authentication.as_str(),
            "Setting shared secret (16 bytes)"
        );
        self.shared_secret = secret;
    }
}
