pub mod passthrough;
pub mod offline;
pub mod status;
pub mod client_only;

use crate::core::actors::client::MinecraftClient;
use crate::core::actors::server::MinecraftServer;
use crate::core::event::MinecraftCommunication;
use crate::network::connection::PossibleReadValue;
use crate::server::ServerResponse;
use crate::version::Version;
use crate::{core::actors::server::MinecraftServerHandler, network::connection::Connection};
use client_only::ClientOnlyMode;
use offline::OfflineMode;
use passthrough::PassthroughMode;
use serde::{Deserialize, Serialize};
use std::io;

#[async_trait::async_trait]
pub trait ClientProxyModeHandler<T>: Send + Sync {
    async fn initialize_client(&self, actor: &mut MinecraftClient<T>) -> io::Result<()>;

    async fn handle_internal_client(
        &self,
        message: T,
        actor: &mut MinecraftClient<T>,
    ) -> io::Result<()>;

    // External TCP stream handlers
    async fn handle_external_client(
        &self,
        data: PossibleReadValue,
        actor: &mut MinecraftClient<T>,
    ) -> io::Result<()>;
}

#[async_trait::async_trait]
pub trait ServerProxyModeHandler<T>: Send + Sync {
    async fn initialize_server(&self, actor: &mut MinecraftServer<T>) -> io::Result<()>;

    async fn handle_external_server(
        &self,
        data: PossibleReadValue,
        actor: &mut MinecraftServer<T>,
    ) -> io::Result<()>;

    async fn handle_internal_server(
        &self,
        message: T,
        actor: &mut MinecraftServer<T>,
    ) -> io::Result<()>;
}
pub trait ProxyMessage: Send + Sync {}

pub trait ProxyModeMessageType {
    type Message: ProxyMessage;
}

pub fn get_proxy_mode<T: ProxyMessage>(
    mode: ProxyModeEnum,
) -> (
    Box<dyn ClientProxyModeHandler<T>>,
    Box<dyn ServerProxyModeHandler<T>>,
)
where
    PassthroughMode: ProxyModeMessageType<Message = T>,
    PassthroughMode: ClientProxyModeHandler<T>,
    PassthroughMode: ServerProxyModeHandler<T>,
    OfflineMode: ProxyModeMessageType<Message = T>,
    OfflineMode: ClientProxyModeHandler<T>,
    OfflineMode: ServerProxyModeHandler<T>,
    ClientOnlyMode: ProxyModeMessageType<Message = T>,
    ClientOnlyMode: ClientProxyModeHandler<T>,
    ClientOnlyMode: ServerProxyModeHandler<T>,
{
    match mode {
        ProxyModeEnum::Passthrough => (Box::new(PassthroughMode), Box::new(PassthroughMode)),
        ProxyModeEnum::Offline => (Box::new(OfflineMode), Box::new(OfflineMode)),
        ProxyModeEnum::ClientOnly => (Box::new(ClientOnlyMode), Box::new(ClientOnlyMode)),
        ProxyModeEnum::ServerOnly => (Box::new(PassthroughMode), Box::new(PassthroughMode)),
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone, Default)]
pub enum ProxyModeEnum {
    #[serde(rename = "passthrough")]
    #[default]
    Passthrough,
    // #[serde(rename = "full")]
    // Full,
    #[serde(rename = "client_only")]
    ClientOnly,
    #[serde(rename = "offline")]
    Offline,
    #[serde(rename = "server_only")]
    ServerOnly,
}
