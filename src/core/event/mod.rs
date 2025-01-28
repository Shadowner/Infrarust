use std::collections::HashMap;

use crate::network::packet::Packet;

use super::config::ServerConfig;

#[derive(Debug, Clone)]
pub enum GatewayMessage {
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum ProviderMessage {
    Update {
        key: String,
        configuration: Option<ServerConfig>,
    },
    FirstInit(HashMap<String, ServerConfig>),
    Error(String),
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum MinecraftCommunication<T> {
    RawData(Vec<u8>),
    Packet(Packet),
    Shutdown,
    CustomData(T),
}
