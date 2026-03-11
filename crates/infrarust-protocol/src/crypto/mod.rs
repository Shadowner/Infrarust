//! Cryptographic primitives for Minecraft protocol encryption.
//!
//! Provides AES-128-CFB8 ciphers used by Minecraft's encryption layer.
//! These are used independently by the transport layer — not by the
//! encoder/decoder, since a proxy manages two separate crypto tunnels.

pub mod cipher;

pub use cipher::{DecryptCipher, EncryptCipher};
