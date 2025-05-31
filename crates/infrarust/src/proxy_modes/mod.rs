pub mod client_only;
pub mod offline;
pub mod passthrough;
pub mod status;

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

//TODO: Refacor to remove the warning
#[allow(clippy::type_complexity)]
#[instrument(name = "create_passthrough_mode")]
pub fn get_passthrough_mode() -> (
    Box<dyn ClientProxyModeHandler<MinecraftCommunication<PassthroughMessage>>>,
    Box<dyn ServerProxyModeHandler<MinecraftCommunication<PassthroughMessage>>>,
) {
    debug!(
        log_type = LogType::ProxyMode.as_str(),
        "Creating new passthrough mode handler pair"
    );
    (Box::new(PassthroughMode), Box::new(PassthroughMode))
}

#[allow(clippy::type_complexity)]
#[instrument(name = "create_offline_mode")]
pub fn get_offline_mode() -> (
    Box<dyn ClientProxyModeHandler<MinecraftCommunication<OfflineMessage>>>,
    Box<dyn ServerProxyModeHandler<MinecraftCommunication<OfflineMessage>>>,
) {
    debug!(
        log_type = LogType::ProxyMode.as_str(),
        "Creating new offline mode handler pair"
    );
    (Box::new(OfflineMode), Box::new(OfflineMode))
}

#[allow(clippy::type_complexity)]
#[instrument(name = "create_client_only_mode")]
pub fn get_client_only_mode() -> (
    Box<dyn ClientProxyModeHandler<MinecraftCommunication<ClientOnlyMessage>>>,
    Box<dyn ServerProxyModeHandler<MinecraftCommunication<ClientOnlyMessage>>>,
) {
    debug!(
        log_type = LogType::ProxyMode.as_str(),
        "Creating new client-only mode handler pair"
    );
    (Box::new(ClientOnlyMode), Box::new(ClientOnlyMode))
}

#[allow(clippy::type_complexity)]
#[instrument(name = "create_status_mode")]
pub fn get_status_mode() -> (
    Box<dyn ClientProxyModeHandler<MinecraftCommunication<StatusMessage>>>,
    Box<dyn ServerProxyModeHandler<MinecraftCommunication<StatusMessage>>>,
) {
    debug!(
        log_type = LogType::ProxyMode.as_str(),
        "Creating new status mode handler pair"
    );
    (Box::new(status::StatusMode), Box::new(status::StatusMode))
}
