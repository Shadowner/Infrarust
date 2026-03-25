//! AES-128-CFB8 encryption and decryption for Minecraft protocol.
//!
//! Minecraft uses AES-128 in CFB8 mode with the shared secret as both
//! key and IV. The ciphers are stateful stream ciphers — each call
//! continues where the previous one left off.

use aes::Aes128;
use cfb8::Decryptor as Cfb8Decryptor;
use cfb8::Encryptor as Cfb8Encryptor;
use cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};

/// Encrypts a byte stream using AES-128-CFB8.
///
/// Stateful: each call to [`encrypt`](Self::encrypt) continues where the
/// previous one left off. Bytes must be encrypted in the exact order they
/// will be sent on the socket.
pub struct EncryptCipher {
    inner: Cfb8Encryptor<Aes128>,
}

/// Decrypts a byte stream using AES-128-CFB8.
///
/// Stateful: each call to [`decrypt`](Self::decrypt) continues where the
/// previous one left off. Bytes must be decrypted in the exact order they
/// were received from the socket.
pub struct DecryptCipher {
    inner: Cfb8Decryptor<Aes128>,
}

impl EncryptCipher {
    /// Creates an encryption cipher from a 16-byte shared secret.
    ///
    /// The IV is set equal to the key, as per the Minecraft protocol.
    ///
    /// # Panics
    /// Cannot panic: the key and IV are always exactly 16 bytes.
    #[allow(clippy::expect_used)]
    pub fn new(key: &[u8; 16]) -> Self {
        Self {
            inner: Cfb8Encryptor::<Aes128>::new_from_slices(key, key)
                .expect("key and iv are always 16 bytes"),
        }
    }

    /// Encrypts data in-place.
    ///
    /// The cipher state advances by `data.len()` bytes.
    pub fn encrypt(&mut self, data: &mut [u8]) {
        // CFB8 has BlockSize=U1, so we process byte-by-byte via BlockEncryptMut
        for byte in data.iter_mut() {
            let mut block = cipher::generic_array::GenericArray::from([*byte]);
            self.inner.encrypt_block_mut(&mut block);
            *byte = block[0];
        }
    }
}

impl DecryptCipher {
    /// Creates a decryption cipher from a 16-byte shared secret.
    ///
    /// The IV is set equal to the key, as per the Minecraft protocol.
    ///
    /// # Panics
    /// Cannot panic: the key and IV are always exactly 16 bytes.
    #[allow(clippy::expect_used)]
    pub fn new(key: &[u8; 16]) -> Self {
        Self {
            inner: Cfb8Decryptor::<Aes128>::new_from_slices(key, key)
                .expect("key and iv are always 16 bytes"),
        }
    }

    /// Decrypts data in-place.
    ///
    /// The cipher state advances by `data.len()` bytes.
    pub fn decrypt(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            let mut block = cipher::generic_array::GenericArray::from([*byte]);
            self.inner.decrypt_block_mut(&mut block);
            *byte = block[0];
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_encrypt_decrypt_round_trip() {
        let key = [0x42u8; 16];
        let original = b"Hello, Minecraft!";
        let mut data = original.to_vec();

        let mut enc = EncryptCipher::new(&key);
        enc.encrypt(&mut data);

        let mut dec = DecryptCipher::new(&key);
        dec.decrypt(&mut data);

        assert_eq!(&data, original);
    }

    #[test]
    fn test_encrypt_changes_data() {
        let key = [0x01u8; 16];
        let original = b"plaintext data here";
        let mut data = original.to_vec();

        let mut enc = EncryptCipher::new(&key);
        enc.encrypt(&mut data);

        assert_ne!(&data[..], &original[..]);
    }

    #[test]
    fn test_stateful_encryption() {
        let key = [0xABu8; 16];
        let data_a = b"first chunk";
        let data_b = b"second chunk";

        // Encrypt in two separate calls
        let mut enc1 = EncryptCipher::new(&key);
        let mut a = data_a.to_vec();
        let mut b = data_b.to_vec();
        enc1.encrypt(&mut a);
        enc1.encrypt(&mut b);

        // Encrypt as one concatenated call
        let mut enc2 = EncryptCipher::new(&key);
        let mut combined = [data_a.as_slice(), data_b.as_slice()].concat();
        enc2.encrypt(&mut combined);

        assert_eq!(&a[..], &combined[..data_a.len()]);
        assert_eq!(&b[..], &combined[data_a.len()..]);
    }

    #[test]
    fn test_different_keys_different_output() {
        let data = b"same input";

        let mut out1 = data.to_vec();
        EncryptCipher::new(&[0x01; 16]).encrypt(&mut out1);

        let mut out2 = data.to_vec();
        EncryptCipher::new(&[0x02; 16]).encrypt(&mut out2);

        assert_ne!(out1, out2);
    }

    #[test]
    fn test_encrypt_empty_data() {
        let mut enc = EncryptCipher::new(&[0x00; 16]);
        let mut data = vec![];
        enc.encrypt(&mut data);
        assert!(data.is_empty());
    }

    #[test]
    fn test_encrypt_single_byte() {
        let key = [0xFFu8; 16];
        let mut data = vec![0x42];

        let mut enc = EncryptCipher::new(&key);
        enc.encrypt(&mut data);

        let mut dec = DecryptCipher::new(&key);
        dec.decrypt(&mut data);

        assert_eq!(data, vec![0x42]);
    }

    #[test]
    fn test_encrypt_large_data() {
        let key = [0x77u8; 16];
        let original: Vec<u8> = (0..1_000_000).map(|i: u32| (i % 256) as u8).collect();
        let mut data = original.clone();

        let mut enc = EncryptCipher::new(&key);
        enc.encrypt(&mut data);

        let mut dec = DecryptCipher::new(&key);
        dec.decrypt(&mut data);

        assert_eq!(data, original);
    }

    #[test]
    fn test_wrong_key_cannot_decrypt() {
        let key1 = [0x11u8; 16];
        let key2 = [0x22u8; 16];
        let original = b"secret message here";
        let mut data = original.to_vec();

        let mut enc = EncryptCipher::new(&key1);
        enc.encrypt(&mut data);

        let mut dec = DecryptCipher::new(&key2);
        dec.decrypt(&mut data);

        assert_ne!(&data[..], &original[..]);
    }

    #[test]
    fn test_two_independent_ciphers() {
        let key = [0x33u8; 16];
        let original = b"independent test";

        let mut data1 = original.to_vec();
        let mut data2 = original.to_vec();

        EncryptCipher::new(&key).encrypt(&mut data1);
        EncryptCipher::new(&key).encrypt(&mut data2);

        assert_eq!(data1, data2);
    }
}
