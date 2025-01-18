use crate::protocol::types::{ProtocolRead, ProtocolString, ProtocolWrite};
use serde::{Deserialize, Serialize};
use std::io;

pub const CLIENTBOUND_RESPONSE_ID: i32 = 0x00;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientBoundResponse {
    pub json_response: ProtocolString,
}

impl ProtocolWrite for ClientBoundResponse {
    fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        self.json_response.write_to(writer)
    }
}

impl ProtocolRead for ClientBoundResponse {
    fn read_from<R: io::Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let (json_response, n) = ProtocolString::read_from(reader)?;
        Ok((Self { json_response }, n))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseJSON {
    pub version: VersionJSON,
    pub players: PlayersJSON,
    pub description: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
    #[serde(default)]
    pub previews_chat: bool,
    #[serde(default)]
    pub enforces_secure_chat: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modinfo: Option<FMLModInfoJSON>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forge_data: Option<FML2ForgeDataJSON>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionJSON {
    pub name: String,
    pub protocol: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayersJSON {
    pub max: i32,
    pub online: i32,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub sample: Vec<PlayerSampleJSON>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerSampleJSON {
    pub name: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DescriptionJSON {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FMLModInfoJSON {
    #[serde(rename = "type")]
    pub loader_type: String,
    #[serde(rename = "modList")]
    pub mod_list: Vec<FMLModJSON>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FMLModJSON {
    #[serde(rename = "modid")]
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FML2ForgeDataJSON {
    pub channels: Vec<FML2ChannelsJSON>,
    pub mods: Vec<FML2ModJSON>,
    #[serde(rename = "fmlNetworkVersion")]
    pub fml_network_version: i32,
    pub d: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FML2ChannelsJSON {
    pub res: String,
    pub version: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FML2ModJSON {
    #[serde(rename = "modId")]
    pub id: String,
    pub modmarker: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_response_serialization() {
        let response = ResponseJSON {
            version: VersionJSON {
                name: "1.19.2".to_string(),
                protocol: 760,
            },
            players: PlayersJSON {
                max: 100,
                online: 50,
                sample: vec![PlayerSampleJSON {
                    name: "Steve".to_string(),
                    id: "uuid".to_string(),
                }],
            },
            description: json!({ "text": "A Minecraft Server" }),
            favicon: Some("data:image/png;base64,...".to_string()),
            previews_chat: true,
            enforces_secure_chat: true,
            modinfo: None,
            forge_data: None,
        };

        let json_str = serde_json::to_string(&response).unwrap();
        let deserialized: ResponseJSON = serde_json::from_str(&json_str).unwrap();
        assert_eq!(response, deserialized);
    }

    #[test]
    fn test_forge_data_serialization() {
        let forge_data = FML2ForgeDataJSON {
            channels: vec![FML2ChannelsJSON {
                res: "minecraft:channel".to_string(),
                version: "1.0".to_string(),
                required: true,
            }],
            mods: vec![FML2ModJSON {
                id: "forge".to_string(),
                modmarker: "marker".to_string(),
            }],
            fml_network_version: 1,
            d: "test".to_string(),
        };

        let json_str = serde_json::to_string(&forge_data).unwrap();
        let deserialized: FML2ForgeDataJSON = serde_json::from_str(&json_str).unwrap();
        assert_eq!(forge_data, deserialized);
    }
}
