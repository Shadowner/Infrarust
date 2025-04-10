use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::api::{ApiProvider, ApiServerStatus};
use crate::error::ServerManagerError;
use crate::ServerState;

#[derive(Clone)]
pub struct MockApiProvider {
    server_states: Arc<RwLock<HashMap<String, ApiServerStatus>>>,
}

impl Default for MockApiProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MockApiProvider {
    pub fn new() -> Self {
        Self {
            server_states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn with_server(self, server_id: &str, status: ApiServerStatus) -> Self {
        self.server_states
            .write()
            .unwrap()
            .insert(server_id.to_string(), status);
        self
    }

    pub fn set_server_crashed(&self, server_id: &str) {
        let mut states = self.server_states.write().unwrap();
        if let Some(status) = states.get_mut(server_id) {
            status.is_running = false;
            status.is_crashed = true;
            status.status = ServerState::Crashed;
            status.error = Some("Server has crashed".to_string());
        }
    }

    pub fn set_server_running(&self, server_id: &str) {
        let mut states = self.server_states.write().unwrap();
        if let Some(status) = states.get_mut(server_id) {
            status.is_running = true;
            status.is_crashed = false;
            status.status = ServerState::Running;
            status.error = None;
        }
    }

    pub fn set_server_stopped(&self, server_id: &str) {
        let mut states = self.server_states.write().unwrap();
        if let Some(status) = states.get_mut(server_id) {
            status.is_running = false;
            status.is_crashed = false;
            status.status = ServerState::Stopped;
            status.error = None;
        }
    }
}

#[async_trait]
impl ApiProvider for MockApiProvider {
    async fn get_server_status(
        &self,
        server_id: &str,
    ) -> Result<ApiServerStatus, ServerManagerError> {
        let states = self.server_states.read().unwrap();
        match states.get(server_id) {
            Some(status) => Ok(status.clone()),
            None => Err(ServerManagerError::ApiError(format!(
                "Server {} not found",
                server_id
            ))),
        }
    }

    async fn start_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        let states = self.server_states.read().unwrap();
        if !states.contains_key(server_id) {
            return Err(ServerManagerError::ApiError(format!(
                "Server {} not found",
                server_id
            )));
        }
        drop(states);

        self.set_server_running(server_id);
        Ok(())
    }

    async fn stop_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        let states = self.server_states.read().unwrap();
        if !states.contains_key(server_id) {
            return Err(ServerManagerError::ApiError(format!(
                "Server {} not found",
                server_id
            )));
        }
        drop(states);

        self.set_server_stopped(server_id);
        Ok(())
    }
}
