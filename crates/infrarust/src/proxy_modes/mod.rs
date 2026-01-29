pub mod client_only;
pub mod offline;
pub mod passthrough;
pub mod status;
pub mod zerocopy;

use crate::core::actors::server::MinecraftServer;
use crate::core::{actors::client::MinecraftClient, event::MinecraftCommunication};
use crate::network::connection::PossibleReadValue;
use client_only::{ClientOnlyMessage, ClientOnlyMode};
use infrarust_config::LogType;
use offline::{OfflineMessage, OfflineMode};
use passthrough::{PassthroughMessage, PassthroughMode};
use status::StatusMessage;
use std::io;
use tracing::{debug, instrument};
pub use zerocopy::{ZeroCopyMessage, spawn_splice_task};

pub type ClientHandler<T> = Box<dyn ClientProxyModeHandler<MinecraftCommunication<T>>>;
pub type ServerHandler<T> = Box<dyn ServerProxyModeHandler<MinecraftCommunication<T>>>;
pub type ProxyModePair<T> = (ClientHandler<T>, ServerHandler<T>);
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

#[instrument(name = "create_passthrough_mode")]
pub fn get_passthrough_mode() -> ProxyModePair<PassthroughMessage> {
    debug!(
        log_type = LogType::ProxyMode.as_str(),
        "Creating new passthrough mode handler pair"
    );
    (Box::new(PassthroughMode), Box::new(PassthroughMode))
}

#[instrument(name = "create_offline_mode")]
pub fn get_offline_mode() -> ProxyModePair<OfflineMessage> {
    debug!(
        log_type = LogType::ProxyMode.as_str(),
        "Creating new offline mode handler pair"
    );
    (Box::new(OfflineMode), Box::new(OfflineMode))
}

#[instrument(name = "create_client_only_mode")]
pub fn get_client_only_mode() -> ProxyModePair<ClientOnlyMessage> {
    debug!(
        log_type = LogType::ProxyMode.as_str(),
        "Creating new client-only mode handler pair"
    );
    (Box::new(ClientOnlyMode), Box::new(ClientOnlyMode))
}

#[instrument(name = "create_status_mode")]
pub fn get_status_mode() -> ProxyModePair<StatusMessage> {
    debug!(
        log_type = LogType::ProxyMode.as_str(),
        "Creating new status mode handler pair"
    );
    (Box::new(status::StatusMode), Box::new(status::StatusMode))
}
