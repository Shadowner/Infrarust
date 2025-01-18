// /!\ This file is not used in the project /!\

// First Idea to implément the compression system onto the packet system
// After some internal debate I decided to not implement encryption and compression onto the packet system

// use super::{CompressionState, Packet, PacketError, PacketResult};
// use crate::{
//     protocol::types::{VarInt, WriteToBytes},
//     ProtocolWrite,
// };
// use bytes::BytesMut;
// use log::{debug, error};
// use std::io::{Read, Write};

// /// Trait pour la compression/décompression des paquets
// pub trait PacketCompression {
//     /// Compresse les données du paquet si nécessaire
//     fn compress(&self) -> PacketResult<BytesMut>;

//     /// Décompresse les données du paquet
//     fn decompress(&self) -> PacketResult<BytesMut>;

//     /// Détermine si les données doivent être compressées
//     fn should_compress(&self) -> bool;
// }

// impl PacketCompression for Packet {
//     fn compress(&self) -> PacketResult<BytesMut> {
//         match self.compression {
//             CompressionState::Disabled => {
//                 debug!("Compression disabled, returning raw data");
//                 Ok(self.data.clone())
//             }
//             CompressionState::Enabled { threshold } => {
//                 // Prepare full packet data
//                 let mut uncompressed_packet = Vec::new();
//                 VarInt(self.id)
//                     .write_to(&mut uncompressed_packet)
//                     .map_err(PacketError::Io)?;
//                 uncompressed_packet.extend_from_slice(&self.data);
//                 let uncompressed_size = uncompressed_packet.len();

//                 debug!(
//                     "Preparing compression: size={}, threshold={}, packet_id=0x{:x}",
//                     uncompressed_size, threshold, self.id
//                 );

//                 if uncompressed_size > MAX_UNCOMPRESSED_SIZE {
//                     return Err(PacketError::compression("Packet too large"));
//                 }

//                 let mut output = BytesMut::new();

//                 if uncompressed_size < threshold as usize {
//                     // Send uncompressed with Data Length = 0
//                     debug!("Below threshold, sending uncompressed");
//                     VarInt(0).write_to_bytes(&mut output)?;
//                     output.extend_from_slice(&uncompressed_packet);
//                 } else {
//                     debug!("Above threshold, compressing data");
//                     let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
//                     encoder.write_all(&uncompressed_packet)?;
//                     let compressed = encoder.finish()?;

//                     VarInt(uncompressed_size as i32).write_to_bytes(&mut output)?;

//                     // Write compressed payload
//                     output.extend_from_slice(&compressed);

//                     debug!(
//                         "Compressed: original={}, compressed={}, ratio={:.2}",
//                         uncompressed_size,
//                         compressed.len(),
//                         compressed.len() as f32 / uncompressed_size as f32
//                     );
//                 }

//                 Ok(output)
//             }
//         }
//     }

//     fn decompress(&self) -> PacketResult<BytesMut> {
//         match self.compression {
//             CompressionState::Disabled => Ok(self.data.clone()),
//             CompressionState::Enabled { threshold } => {
//                 // Read Data Length
//                 let mut uncompressed_packet = Vec::new();
//                 VarInt(self.id)
//                     .write_to(&mut uncompressed_packet)
//                     .map_err(PacketError::Io)?;
//                 uncompressed_packet.extend_from_slice(&self.data);
//                 let uncompressed_size = uncompressed_packet.len();

//                 debug!(
//                     "Decompressing packet: declared length: {}, threshold: {}",
//                     uncompressed_size, threshold
//                 );

//                 if uncompressed_size == 0 || uncompressed_size < threshold as usize {
//                     // Uncompressed packet - return raw data after Data Length
//                     let data = &self.data;
//                     let mut output = BytesMut::with_capacity(data.len());
//                     output.extend_from_slice(data);
//                     debug!("Uncompressed packet: size={}", data.len());
//                     return Ok(output);
//                 }

//                 // Verify size requirements for compressed packets
//                 if uncompressed_size as usize > MAX_UNCOMPRESSED_SIZE {
//                     return Err(PacketError::compression("Invalid declared length"));
//                 }

//                 if uncompressed_size < threshold as usize {
//                     return Err(PacketError::compression(
//                         "Compressed packet smaller than threshold",
//                     ));
//                 }

//                 // Decompress
//                 let compressed = &self.data;
//                 debug!("Compressed data size: {}", compressed.len());

//                 let mut decoder = ZlibDecoder::new(std::io::Cursor::new(&compressed[..]));
//                 let mut decompressed = Vec::with_capacity(uncompressed_size);

//                 if let Err(e) = decoder.read_to_end(&mut decompressed) {
//                     error!("Decompression error: {}", e);
//                     return Err(PacketError::compression(format!(
//                         "Decompression failed: {}",
//                         e
//                     )));
//                 }

//                 // Verify decompressed length
//                 if decompressed.len() != uncompressed_size {
//                     error!(
//                         "Length mismatch: expected={}, got={}",
//                         uncompressed_size,
//                         decompressed.len()
//                     );
//                     return Err(PacketError::compression("Length mismatch"));
//                 }

//                 let mut output = BytesMut::with_capacity(decompressed.len());
//                 output.extend_from_slice(&decompressed);
//                 Ok(output)
//             }
//         }
//     }
//     fn should_compress(&self) -> bool {
//         match self.compression {
//             CompressionState::Disabled => false,
//             CompressionState::Enabled { threshold } => {
//                 threshold >= 0 && self.data.len() >= threshold as usize
//             }
//         }
//     }
// }

// #[cfg(test)]
// mod tests {
//     use crate::ProtocolRead;

//     use super::*;

//     #[test]
//     fn test_compression_disabled() {
//         let mut packet = Packet::new(0x00);
//         packet.data.extend_from_slice(b"test data");

//         let compressed = packet.compress().unwrap();
//         assert_eq!(&compressed[..], b"test data");

//         let decompressed = packet.decompress().unwrap();
//         assert_eq!(&decompressed[..], b"test data");
//     }

//     #[test]
//     fn test_compression_above_threshold() {
//         let mut packet = Packet::new(0x00);
//         packet.enable_compression(256);

//         // Créer des données qui dépassent le seuil
//         let data = vec![1u8; 1000];
//         packet.data = BytesMut::from(&data[..]);

//         let compressed = packet.compress().unwrap();
//         let mut compressed_packet = packet.clone();
//         compressed_packet.data = compressed;

//         let decompressed = compressed_packet.decompress().unwrap();

//         // Vérifier que l'ID du paquet et les données sont préservés
//         let mut cursor = std::io::Cursor::new(&decompressed[..]);
//         let (VarInt(id), _) = VarInt::read_from(&mut cursor).unwrap();
//         assert_eq!(id, packet.id);

//         let remaining_data = &decompressed[cursor.position() as usize..];
//         assert_eq!(remaining_data, &data[..]);
//     }

//     #[test]
//     fn test_compression_below_threshold() {
//         let mut packet = Packet::new(0x00);
//         packet.enable_compression(256);

//         let data = vec![1u8; 100]; // En dessous du seuil
//         packet.data = BytesMut::from(&data[..]);

//         let compressed = packet.compress().unwrap();
//         assert_eq!(compressed[0], 0); // VarInt(0) indiquant pas de compression

//         let mut compressed_packet = packet.clone();
//         compressed_packet.data = compressed;

//         let decompressed = compressed_packet.decompress().unwrap();
//         assert_eq!(&decompressed[..], &packet.data[..]);
//     }

//     #[test]
//     #[ignore]
//     // TODO: fix test
//     fn test_invalid_compressed_length() {
//         let mut packet_to_compress = Packet::new(0x00);
//         packet_to_compress.enable_compression(0);

//         let mut data = Vec::new();

//         // 8388609 = 0x800001 = 0b100000000000000000001
//         // Proper 3-byte VarInt encoding:
//         data.push(0x81); // 0b10000001: LSB 1 + continuation
//         data.push(0x86); // 0b10000000: middle byte 0 + continuation
//         data.push(0x40); // 0b01000000: MSB 1, no continuation

//         packet_to_compress.data = BytesMut::from(&data[..]);
//         let compressed_packet = packet_to_compress.compress();

//         let data = compressed_packet.unwrap();

//         let mut packet_to_decompress = Packet::new(0x00);
//         packet_to_decompress.enable_compression(0);
//         packet_to_decompress.data = BytesMut::from(&data[..]);

//         let result = packet_to_decompress.decompress();
//         println!("Result: {:?}", result);

//         assert!(result.is_err());
//     }

//     #[test]
//     fn test_max_uncompressed_size() {
//         let mut packet = Packet::new(0x00);
//         packet.enable_compression(0);

//         // Test avec une taille exactement égale à la limite
//         let mut temp = Vec::new();
//         VarInt(MAX_UNCOMPRESSED_SIZE as i32)
//             .write_to(&mut temp)
//             .expect_err("Should fail");

//         // Vérifier que l'erreur est bien due à la taille du VarInt
//         packet.data = BytesMut::from(&temp[..]);
//         assert!(packet.decompress().is_err());
//     }
// }
