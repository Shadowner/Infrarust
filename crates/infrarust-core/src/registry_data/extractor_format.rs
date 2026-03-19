//! Serialization format for extracted registry data.

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// Magic bytes identifying a registry extraction file.
pub const MAGIC: &[u8; 4] = b"IREG";

/// Current format version.
pub const FORMAT_VERSION: u8 = 1;

/// Registry data extracted for a single protocol version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedRegistryData {
    /// Serialization format version.
    pub format_version: u8,
    /// Minecraft protocol version (e.g. 774 for 1.21.11).
    pub protocol_version: i32,
    /// Minecraft version name (e.g. "1.21.11").
    pub minecraft_version: String,
    /// Extraction date (ISO 8601).
    pub extraction_date: String,
    /// CKnownPacks frame (if sent by the server, >= 1.20.5).
    pub known_packs_frame: Option<FrameData>,
    /// CRegistryData frames in send order.
    pub registry_frames: Vec<FrameData>,
}

/// A serialized packet frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameData {
    /// Packet ID as sent by the server.
    pub packet_id: i32,
    /// Full packet payload (without frame length or packet ID).
    pub payload: Vec<u8>,
}

impl ExtractedRegistryData {
    /// Serialize to JSON (used by the registry-extractor tool).
    #[allow(dead_code)]
    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Deserialize from JSON.
    pub fn from_json(data: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(data)
    }

    /// Serialize to compact binary format (used by the registry-extractor tool).
    ///
    /// Layout: `MAGIC (4 bytes) + FORMAT_VERSION (1 byte) + JSON length (4 bytes LE) + JSON bytes`
    #[allow(dead_code)]
    pub fn to_binary(&self) -> Result<Vec<u8>, CoreError> {
        let json = self
            .to_json()
            .map_err(|e| CoreError::Other(e.to_string()))?;
        let mut buf = Vec::with_capacity(9 + json.len());
        buf.extend_from_slice(MAGIC);
        buf.push(FORMAT_VERSION);
        buf.extend_from_slice(&(json.len() as u32).to_le_bytes());
        buf.extend_from_slice(&json);
        Ok(buf)
    }

    /// Deserialize from compact binary format.
    pub fn from_binary(data: &[u8]) -> Result<Self, CoreError> {
        if data.len() < 9 {
            return Err(CoreError::Other(
                "Registry data file too short".into(),
            ));
        }
        if &data[0..4] != MAGIC {
            return Err(CoreError::Other(
                "Invalid registry data magic".into(),
            ));
        }
        let version = data[4];
        if version != FORMAT_VERSION {
            return Err(CoreError::Other(format!(
                "Unsupported registry data format version: {version}"
            )));
        }
        let json_len = u32::from_le_bytes([data[5], data[6], data[7], data[8]]) as usize;
        if data.len() < 9 + json_len {
            return Err(CoreError::Other(
                "Registry data file truncated".into(),
            ));
        }
        Self::from_json(&data[9..9 + json_len])
            .map_err(|e| CoreError::Other(format!("Registry data JSON parse error: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_roundtrip() {
        let data = ExtractedRegistryData {
            format_version: FORMAT_VERSION,
            protocol_version: 774,
            minecraft_version: "1.21.11".into(),
            extraction_date: "2026-03-19T12:00:00Z".into(),
            known_packs_frame: Some(FrameData {
                packet_id: 0x0E,
                payload: vec![0x01, 0x02, 0x03],
            }),
            registry_frames: vec![FrameData {
                packet_id: 0x07,
                payload: vec![0x0A, 0x0B, 0x0C],
            }],
        };

        let binary = data.to_binary().unwrap();
        assert_eq!(&binary[0..4], MAGIC);
        assert_eq!(binary[4], FORMAT_VERSION);

        let restored = ExtractedRegistryData::from_binary(&binary).unwrap();
        assert_eq!(restored.protocol_version, 774);
        assert_eq!(restored.minecraft_version, "1.21.11");
        assert_eq!(restored.registry_frames.len(), 1);
        assert!(restored.known_packs_frame.is_some());
    }

    #[test]
    fn test_invalid_magic() {
        let data = vec![0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02];
        assert!(ExtractedRegistryData::from_binary(&data).is_err());
    }

    #[test]
    fn test_truncated_data() {
        let data = vec![b'I', b'R', b'E', b'G', 0x01];
        assert!(ExtractedRegistryData::from_binary(&data).is_err());
    }
}
