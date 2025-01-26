use std::collections::HashMap;

use crate::network::packet::Packet;

use super::config::ServerConfig;


#[derive(Debug, Clone)]
pub enum GatewayMessage {
    ConfigurationUpdate {
        key: String,
        configuration: ServerConfig,
    },
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum ProviderMessage {
    Update {
        key: String,
        configuration: ServerConfig,
    },
    FirstInit(HashMap<String, ServerConfig>),
    Error(String),
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum MinecraftCommunication {
    RawData(Vec<u8>),
    Packet(Packet),
    Shutdown,
}