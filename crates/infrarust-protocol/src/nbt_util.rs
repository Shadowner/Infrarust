//! NBT utilities for the Minecraft network protocol.
//!
//! Since Minecraft 1.20.2 (protocol 764), NBT sent over the network uses a
//! "nameless root compound" format: the root compound tag has no name.
//! This module provides wrappers around `fastnbt` to produce this format.
//!
//! For the existing NBT *skipper* (read-only, no parsing), see [`crate::nbt`].

use serde::Serialize;

/// Serializes a value as Network NBT (nameless root compound).
///
/// Since 1.20.2, network NBT omits the root compound name.
/// `fastnbt::to_bytes()` produces `0x0A 0x00 0x00 [payload] 0x00` (root name = "").
/// This function produces `0x0A [payload] 0x00` (no name length bytes).
///
/// For standard NBT (files, chunks pre-1.20.2), use `fastnbt::to_bytes()` directly.
pub fn to_network_nbt<T: Serialize>(value: &T) -> Result<Vec<u8>, fastnbt::error::Error> {
    let mut bytes = fastnbt::to_bytes(value)?;
    // Standard NBT:  0x0A 0x00 0x00 [compound payload] 0x00
    // Network NBT:   0x0A [compound payload] 0x00
    // Remove the 2 name-length bytes at index 1 and 2.
    if bytes.len() >= 3 && bytes[0] == 0x0A {
        bytes.drain(1..3);
    }
    Ok(bytes)
}

pub use fastnbt::LongArray;

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct SimpleCompound {
        name: String,
        value: i32,
    }

    #[test]
    fn test_to_network_nbt_removes_root_name() {
        let data = SimpleCompound {
            name: "test".into(),
            value: 42,
        };

        let standard = fastnbt::to_bytes(&data).unwrap();
        let network = to_network_nbt(&data).unwrap();

        // Standard: 0x0A 0x00 0x00 [payload]
        assert_eq!(standard[0], 0x0A); // TAG_Compound
        assert_eq!(standard[1], 0x00); // name length high byte
        assert_eq!(standard[2], 0x00); // name length low byte

        // Network: 0x0A [payload] (2 bytes shorter)
        assert_eq!(network[0], 0x0A);
        assert_eq!(network.len(), standard.len() - 2);

        // Content after the header must be identical
        assert_eq!(&network[1..], &standard[3..]);
    }

    #[test]
    fn test_to_network_nbt_empty_struct() {
        #[derive(Serialize)]
        struct Empty {}
        let network = to_network_nbt(&Empty {}).unwrap();
        // Should be: 0x0A 0x00 (compound tag + TAG_End)
        assert_eq!(network[0], 0x0A);
        assert_eq!(*network.last().unwrap(), 0x00);
    }
}
