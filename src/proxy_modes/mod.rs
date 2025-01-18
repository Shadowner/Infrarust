pub mod client_only;
pub mod full;
pub mod offline;
pub mod passthrough;

use crate::network::connection::Connection;
use crate::server::ServerResponse;
use crate::version::Version;
use serde::{Deserialize, Serialize};
use std::io;

#[async_trait::async_trait]
pub trait ProxyModeHandler: Send + Sync {
    async fn handle(
        &self,
        client: Connection,
        response: ServerResponse,
        protocol_version: Version,
    ) -> io::Result<()>;
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone, Default)]
pub enum ProxyModeEnum {
    #[serde(rename = "passthrough")]
    #[default]
    Passthrough,
    #[serde(rename = "full")]
    Full,
    #[serde(rename = "client_only")]
    ClientOnly,
    #[serde(rename = "offline")]
    Offline,
    #[serde(rename = "server_only")]
    ServerOnly,
}
