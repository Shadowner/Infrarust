//! AES-128-CFB8 encryption and decryption for Minecraft protocol.
//!
//! Minecraft uses AES-128 in CFB8 mode with the shared secret as both
//! key and IV. The ciphers are stateful stream ciphers — each call
//! continues where the previous one left off.

use aes::Aes128;
use cfb8::Encryptor as Cfb8Encryptor;
use cipher::{Block, BlockCipherEncrypt, KeyInit, KeyIvInit};

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
    cipher: Aes128,
    /// The last 16 ciphertext bytes seen (initially the IV).
    window: [u8; 16],
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
        self.inner.encrypt(data);
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
            cipher: Aes128::new_from_slice(key).expect("key is always 16 bytes"),
            window: *key,
        }
    }

    /// Decrypts data in-place.
    ///
    /// The cipher state advances by `data.len()` bytes.
    ///
    /// Plaintext byte `i` is `ct[i] ^ AES(K, w_i)[0]` where `w_i` is the 16
    /// ciphertext bytes preceding `i` (seeded by the IV).
    pub fn decrypt(&mut self, data: &mut [u8]) {
        const BATCH: usize = 64;
        let mut keystream = [Block::<Aes128>::default(); BATCH];

        let mut offset = 0;
        while offset < data.len() {
            let n = BATCH.min(data.len() - offset);
            let batch = &data[offset..offset + n];

            for (j, block) in keystream[..n].iter_mut().enumerate() {
                if let Some(start) = j.checked_sub(16) {
                    block.copy_from_slice(&batch[start..j]);
                } else {
                    let carry = 16 - j;
                    block[..carry].copy_from_slice(&self.window[j..]);
                    block[carry..].copy_from_slice(&batch[..j]);
                }
            }

            // Slide the window over the ciphertext before it is overwritten.
            if n >= 16 {
                self.window.copy_from_slice(&batch[n - 16..]);
            } else {
                self.window.copy_within(n.., 0);
                self.window[16 - n..].copy_from_slice(batch);
            }

            self.cipher.encrypt_blocks(&mut keystream[..n]);

            for (byte, ks) in data[offset..offset + n].iter_mut().zip(&keystream) {
                *byte ^= ks[0];
            }
            offset += n;
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

    /// Differential test against the cfb8 crate's reference decryptor,
    /// split across irregular chunk sizes to cross every batch boundary
    /// and exercise the sliding-window state between calls.
    #[test]
    fn test_decrypt_matches_reference_cfb8() {
        use cipher::BlockModeDecrypt;

        let key = [0x5Au8; 16];
        let ciphertext: Vec<u8> = (0..4096u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();

        let mut reference = ciphertext.clone();
        let mut ref_dec = cfb8::Decryptor::<Aes128>::new_from_slices(&key, &key).unwrap();
        for byte in &mut reference {
            let mut block = cipher::Array::from([*byte]);
            ref_dec.decrypt_block(&mut block);
            *byte = block[0];
        }

        let mut ours = ciphertext;
        let mut dec = DecryptCipher::new(&key);
        let mut pos = 0;
        for chunk in [1usize, 7, 15, 16, 17, 33, 63, 64, 65, 100, 512] {
            dec.decrypt(&mut ours[pos..pos + chunk]);
            pos += chunk;
        }
        dec.decrypt(&mut ours[pos..]);

        assert_eq!(ours, reference);
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
