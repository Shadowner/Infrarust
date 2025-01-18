// // First Idea to implement the compression system onto the packet system
// // After some internal debate I decided to not implement encryption and compression onto the packet system




// use aes::cipher::{
//     generic_array::{typenum::U1, GenericArray},
//     inout::InOut,
//     BlockBackend, BlockClosure, BlockDecryptMut, BlockEncryptMut, BlockSizeUser, KeyIvInit,
// };
// use aes::Aes128;
// use bytes::BytesMut;
// use cfb8::{Decryptor, Encryptor};

// use super::{
//     base::{EncryptionState, Packet},
//     error::{PacketError, PacketResult},
// };

// pub type Aes128Cfb8Enc = Encryptor<Aes128>;
// pub type Aes128Cfb8Dec = Decryptor<Aes128>;



// pub struct CipherPair {
//     pub encryptor: Aes128Cfb8Enc,
//     pub decryptor: Aes128Cfb8Dec,
// }

// impl CipherPair {
//     pub fn new(key: &[u8], iv: &[u8]) -> PacketResult<Self> {
//         if key.len() != 16 || iv.len() != 16 {
//             return Err(PacketError::encryption(
//                 "La clé et l'IV doivent faire 16 bytes",
//             ));
//         }

//         let key_array: [u8; 16] = key.try_into().unwrap();
//         let iv_array: [u8; 16] = iv.try_into().unwrap();

//         Ok(Self {
//             encryptor: Aes128Cfb8Enc::new(&key_array.into(), &iv_array.into()),
//             decryptor: Aes128Cfb8Dec::new(&key_array.into(), &iv_array.into()),
//         })
//     }
// }

// pub trait PacketEncryption {
//     fn encrypt(&mut self, cipher: &mut Aes128Cfb8Enc) -> PacketResult<BytesMut>;
//     fn decrypt(&mut self, cipher: &mut Aes128Cfb8Dec) -> PacketResult<BytesMut>;
// }

// // First attempt
// impl PacketEncryption for Packet {
//     fn encrypt(&mut self, cipher: &mut Aes128Cfb8Enc) -> PacketResult<BytesMut> {
//         match self.encryption {
//             EncryptionState::Disabled => Ok(self.data.clone()),
//             EncryptionState::Enabled {
//                 encrypted_data: false,
//             } => {
//                 let mut encrypted = BytesMut::with_capacity(self.data.len());
//                 encrypted.extend_from_slice(&self.data);

//                 cipher.encrypt_with_backend_mut(Cfb8Closure {
//                     data: &mut encrypted,
//                 });

//                 Ok(encrypted)
//             }
//             EncryptionState::Enabled {
//                 encrypted_data: true,
//             } => Err(PacketError::encryption("Les données sont déjà chiffrées")),
//         }
//     }

//     fn decrypt(&mut self, cipher: &mut Aes128Cfb8Dec) -> PacketResult<BytesMut> {
//         match self.encryption {
//             EncryptionState::Disabled => Ok(self.data.clone()),
//             EncryptionState::Enabled {
//                 encrypted_data: true,
//             } => {
//                 let mut decrypted = BytesMut::with_capacity(self.data.len());
//                 decrypted.extend_from_slice(&self.data);

//                 cipher.decrypt_with_backend_mut(Cfb8Closure {
//                     data: &mut decrypted,
//                 });

//                 Ok(decrypted)
//             }
//             EncryptionState::Enabled {
//                 encrypted_data: false,
//             } => Err(PacketError::encryption("Les données ne sont pas chiffrées")),
//         }
//     }
// }

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn test_cipher_operations() {
//         let key = [1u8; 16];
//         let iv = [2u8; 16];
//         let cipher_pair = CipherPair::new(&key, &iv).unwrap();

//         let mut packet = Packet::new(0x00);
//         packet.data.extend_from_slice(b"test data");
//         packet.enable_encryption();

//         let mut encryptor = cipher_pair.encryptor;
//         let mut decryptor = cipher_pair.decryptor;

//         let encrypted = packet.encrypt(&mut encryptor).unwrap();
//         assert_ne!(&encrypted[..], b"test data");

//         let mut encrypted_packet = Packet::new(0x00);
//         encrypted_packet.data = encrypted;
//         encrypted_packet.enable_encryption();
//         encrypted_packet.mark_as_encrypted();

//         let decrypted = encrypted_packet.decrypt(&mut decryptor).unwrap();
//         assert_eq!(&decrypted[..], b"test data");
//     }
// }
