use std::sync::Mutex;

use crate::error::ServiceError;
use crate::event::BoxFuture;
use crate::services::server_manager::{ServerManager, ServerState};
use crate::types::ServerId;

use super::lock;

#[derive(Debug, Default)]
pub struct MockServerManager {
    servers: Mutex<Vec<(ServerId, ServerState)>>,
    started: Mutex<Vec<ServerId>>,
    stopped: Mutex<Vec<ServerId>>,
}

impl MockServerManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_server(self, server: impl Into<ServerId>, state: ServerState) -> Self {
        self.set_state(server, state);
        self
    }

    pub fn set_state(&self, server: impl Into<ServerId>, state: ServerState) {
        let server = server.into();
        let mut servers = lock(&self.servers);
        match servers.iter_mut().find(|(id, _)| *id == server) {
            Some(slot) => slot.1 = state,
            None => servers.push((server, state)),
        }
    }

    #[must_use]
    pub fn started(&self) -> Vec<ServerId> {
        lock(&self.started).clone()
    }

    #[must_use]
    pub fn stopped(&self) -> Vec<ServerId> {
        lock(&self.stopped).clone()
    }

    fn known(&self, server: &ServerId) -> Result<(), ServiceError> {
        self.get_state(server)
            .map(|_| ())
            .ok_or_else(|| ServiceError::NotFound(server.to_string()))
    }
}

impl crate::services::server_manager::private::Sealed for MockServerManager {}

impl ServerManager for MockServerManager {
    fn get_state(&self, server: &ServerId) -> Option<ServerState> {
        lock(&self.servers)
            .iter()
            .find(|(id, _)| id == server)
            .map(|(_, state)| *state)
    }

    fn start(&self, server: &ServerId) -> BoxFuture<'_, Result<(), ServiceError>> {
        let result = self.known(server);
        if result.is_ok() {
            lock(&self.started).push(server.clone());
        }
        Box::pin(async move { result })
    }

    fn stop(&self, server: &ServerId) -> BoxFuture<'_, Result<(), ServiceError>> {
        let result = self.known(server);
        if result.is_ok() {
            lock(&self.stopped).push(server.clone());
        }
        Box::pin(async move { result })
    }

    fn get_all_servers(&self) -> Vec<(ServerId, ServerState)> {
        lock(&self.servers).clone()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::test_util::block_on;

    #[test]
    fn records_lifecycle_calls_for_known_servers_only() {
        let manager = MockServerManager::new().with_server("lobby", ServerState::Offline);
        let lobby = ServerId::new("lobby");
        block_on(manager.start(&lobby)).unwrap();
        manager.set_state("lobby", ServerState::Online);
        block_on(manager.stop(&lobby)).unwrap();
        assert_eq!(manager.started(), std::slice::from_ref(&lobby));
        assert_eq!(manager.stopped(), std::slice::from_ref(&lobby));
        assert_eq!(manager.get_state(&lobby), Some(ServerState::Online));
        assert!(matches!(
            block_on(manager.start(&ServerId::new("nowhere"))),
            Err(ServiceError::NotFound(_))
        ));
        assert_eq!(manager.get_all_servers().len(), 1);
    }
}
