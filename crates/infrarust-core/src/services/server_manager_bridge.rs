//! [`ServerManager`] bridge — delegates to [`ServerManagerService`].

use std::sync::Arc;

use infrarust_api::error::ServiceError;
use infrarust_api::event::{BoxFuture, ListenerHandle};
use infrarust_api::services::server_manager::{ServerManager, ServerState, StateChangeCallback};
use infrarust_api::types::ServerId;

use infrarust_server_manager::service::ServerManagerService;
use infrarust_server_manager::state::ServerState as CoreServerState;

/// Bridges the API-level [`ServerManager`] trait to the core [`ServerManagerService`].
pub struct ServerManagerBridge {
    service: Arc<ServerManagerService>,
}

impl ServerManagerBridge {
    pub fn new(service: Arc<ServerManagerService>) -> Self {
        Self { service }
    }
}

impl infrarust_api::services::server_manager::private::Sealed for ServerManagerBridge {}

impl ServerManager for ServerManagerBridge {
    fn get_state(&self, server: &ServerId) -> Option<ServerState> {
        self.service
            .get_state(server.as_str())
            .map(convert_state)
    }

    fn start(&self, server: &ServerId) -> BoxFuture<'_, Result<(), ServiceError>> {
        let server_id = server.as_str().to_string();
        Box::pin(async move {
            self.service
                .start_server(&server_id)
                .await
                .map_err(|e| ServiceError::OperationFailed(e.to_string()))
        })
    }

    fn stop(&self, server: &ServerId) -> BoxFuture<'_, Result<(), ServiceError>> {
        let server_id = server.as_str().to_string();
        Box::pin(async move {
            self.service
                .stop_server(&server_id)
                .await
                .map_err(|e| ServiceError::OperationFailed(e.to_string()))
        })
    }

    fn on_state_change(&self, callback: StateChangeCallback) -> ListenerHandle {
        let id = self.service.add_on_state_change(Arc::new(
            move |server_id: &str, old: CoreServerState, new: CoreServerState| {
                let server = ServerId::new(server_id);
                callback(&server, convert_state(old), convert_state(new));
            },
        ));

        ListenerHandle::new(id)
    }

    fn get_all_servers(&self) -> Vec<(ServerId, ServerState)> {
        self.service
            .get_all_managed()
            .into_iter()
            .map(|(id, state)| (ServerId::new(id), convert_state(state)))
            .collect()
    }
}

/// A no-op [`ServerManager`] for proxies with no managed servers.
pub struct NoopServerManager;

impl infrarust_api::services::server_manager::private::Sealed for NoopServerManager {}

impl ServerManager for NoopServerManager {
    fn get_state(&self, _server: &ServerId) -> Option<ServerState> {
        None
    }

    fn start(&self, server: &ServerId) -> BoxFuture<'_, Result<(), ServiceError>> {
        let id = server.as_str().to_string();
        Box::pin(async move { Err(ServiceError::NotFound(id)) })
    }

    fn stop(&self, server: &ServerId) -> BoxFuture<'_, Result<(), ServiceError>> {
        let id = server.as_str().to_string();
        Box::pin(async move { Err(ServiceError::NotFound(id)) })
    }

    fn on_state_change(&self, _callback: StateChangeCallback) -> ListenerHandle {
        ListenerHandle::new(0)
    }

    fn get_all_servers(&self) -> Vec<(ServerId, ServerState)> {
        Vec::new()
    }
}

/// Converts core server state to API server state.
fn convert_state(state: CoreServerState) -> ServerState {
    match state {
        CoreServerState::Online => ServerState::Online,
        CoreServerState::Starting => ServerState::Starting,
        CoreServerState::Stopping => ServerState::Stopping,
        CoreServerState::Sleeping => ServerState::Sleeping,
        CoreServerState::Crashed => ServerState::Crashed,
        _ => ServerState::Offline,
    }
}
