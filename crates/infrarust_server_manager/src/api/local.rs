use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::debug;

use crate::api::{ApiProvider, ApiServerStatus};
use crate::error::ServerManagerError;
use crate::process::ProcessProvider;
use crate::{ProcessManager, ServerState};

/// Configuration for a local server
#[derive(Clone, Debug, Deserialize)]
pub struct LocalServerConfig {
    /// The executable to run
    pub executable: String,
    /// Arguments to pass to the executable
    pub args: Vec<String>,
    /// Working directory for the server
    pub working_dir: Option<String>,
    /// String to detect in stdout that marks the server as started
    pub startup_string: Option<String>,
}

/// Provider for managing local server processes
#[derive(Debug, Clone)]
pub struct LocalProvider {
    process_manager: Arc<ProcessManager>,
    configs: Arc<Mutex<HashMap<String, LocalServerConfig>>>,
    server_states: Arc<Mutex<HashMap<String, ServerState>>>,
}

impl LocalProvider {
    pub fn new() -> Self {
        debug!(log_type = "server_manager", "Creating new LocalProvider instance");
        Self {
            process_manager: Arc::new(ProcessManager::new()),
            configs: Arc::new(Mutex::new(HashMap::new())),
            server_states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register_server(&self, server_id: &str, config: LocalServerConfig) {
        debug!(log_type = "server_manager", "Registering server with id: {}", server_id);
        let mut configs = self.configs.lock().unwrap();
        configs.insert(server_id.to_string(), config);
    }

    pub fn unregister_server(&self, server_id: &str) {
        debug!(log_type = "server_manager", "Unregistering server with id: {}", server_id);
        let mut configs = self.configs.lock().unwrap();
        configs.remove(server_id);
    }

    pub fn process_manager(&self) -> Arc<ProcessManager> {
        self.process_manager.clone()
    }
}

#[async_trait]
impl ApiProvider for LocalProvider {
    async fn get_server_status(
        &self,
        server_id: &str,
    ) -> Result<ApiServerStatus, ServerManagerError> {
        debug!(log_type = "server_manager", "Getting status for server: {}", server_id);
        let is_running = self.process_manager.is_process_running(server_id)?;

        let configs = self.configs.lock().unwrap();
        let name = match configs.get(server_id) {
            Some(config) => {
                // Extract filename from the executable path
                Path::new(&config.executable)
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or(server_id)
                    .to_string()
            }
            None => server_id.to_string(),
        };

        let state = if is_running {
            match self.process_manager.get_server_state(server_id) {
                Ok(state) => {
                    let mut server_states = self.server_states.lock().unwrap();
                    server_states.insert(server_id.to_string(), state.clone());
                    state
                }
                Err(_) => {
                    let server_states = self.server_states.lock().unwrap();
                    let state = server_states
                        .get(server_id)
                        .cloned()
                        .unwrap_or(ServerState::Running);
                    state
                }
            }
        } else {
            let mut server_states = self.server_states.lock().unwrap();
            server_states.insert(server_id.to_string(), ServerState::Stopped);
            ServerState::Stopped
        };

        let is_crashed = state == ServerState::Crashed;
        let error = if is_crashed {
            Some("Server has crashed".to_string())
        } else {
            None
        };

        debug!(log_type = "server_manager", "Server {} status: {:?}", server_id, state);
        Ok(ApiServerStatus {
            id: server_id.to_string(),
            name,
            status: state.clone(),
            is_running,
            is_crashed,
            error,
        })
    }

    async fn start_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        debug!(log_type = "server_manager", "Attempting to start server: {}", server_id);
        // Check if already running
        if self.process_manager.is_process_running(server_id)? {
            debug!(log_type = "server_manager", "Server {} is already running", server_id);
            return Ok(());
        }

        // Update server state to Starting
        {
            let mut server_states = self.server_states.lock().unwrap();
            server_states.insert(server_id.to_string(), ServerState::Starting);
        }

        // Get server config
        let config = {
            let configs = self.configs.lock().unwrap();
            match configs.get(server_id) {
                Some(config) => config.clone(),
                None => {
                    debug!(log_type = "server_manager", "No configuration found for server {}", server_id);
                    // Revert state back to Stopped on error
                    let mut server_states = self.server_states.lock().unwrap();
                    server_states.insert(server_id.to_string(), ServerState::Stopped);

                    return Err(ServerManagerError::ProcessError(format!(
                        "No configuration found for server {}",
                        server_id
                    )));
                }
            }
        };

        // Convert args to a slice of &str
        let args: Vec<&str> = config.args.iter().map(|s| s.as_str()).collect();

        debug!(log_type = "server_manager",
            "Starting server {} with executable: {}",
            server_id, config.executable
        );

        // Start the process
        match self.process_manager.start_process(
            server_id,
            &config.executable,
            &args,
            config.working_dir.as_deref(),
            config.startup_string.as_deref(),
        ) {
            Ok(_) => {
                debug!(log_type = "server_manager", "Server {} started successfully", server_id);
                Ok(())
            }
            Err(e) => {
                debug!(log_type = "server_manager", "Failed to start server {}: {}", server_id, e);
                // Revert state back to Stopped on error
                let mut server_states = self.server_states.lock().unwrap();
                server_states.insert(server_id.to_string(), ServerState::Stopped);
                Err(e)
            }
        }
    }

    async fn stop_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        debug!(log_type = "server_manager", "Attempting to stop server: {}", server_id);
        // First try to send a graceful shutdown command (like "stop" or "exit")
        // This may not work for all server types, so we'll also force stop if needed
        debug!(log_type = "server_manager",
            "Sending graceful shutdown commands to server: {}",
            server_id
        );

        // Update server state to Stopping
        {
            let mut server_states = self.server_states.lock().unwrap();
            server_states.insert(server_id.to_string(), ServerState::Stopping);
            debug!(log_type = "server_manager",
                "Set server state for '{}' to Stopping during shutdown",
                server_id
            );
        }

        let _ = self.process_manager.write_stdin(server_id, "stop").await;
        let _ = self.process_manager.write_stdin(server_id, "exit").await;

        // Give it a moment to shut down gracefully
        debug!(log_type = "server_manager", "Waiting for server {} to shut down gracefully", server_id);
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // If still running, force stop it
        if self.process_manager.is_process_running(server_id)? {
            debug!(log_type = "server_manager",
                "Server {} still running after graceful shutdown attempt, forcing stop",
                server_id
            );
            self.process_manager.stop_process(server_id).await?;
        }

        // Update server state to Stopped
        {
            let mut server_states = self.server_states.lock().unwrap();
            server_states.insert(server_id.to_string(), ServerState::Stopped);
            debug!(log_type = "server_manager",
                "Set server state for '{}' to Stopped after shutdown",
                server_id
            );
        }

        debug!(log_type = "server_manager", "Server {} stopped successfully", server_id);
        Ok(())
    }
}

#[async_trait]
impl ProcessProvider for LocalProvider {
    async fn write_stdin(&self, server_id: &str, input: &str) -> Result<(), ServerManagerError> {
        debug!(log_type = "server_manager", "Writing to stdin for server {}: '{}'", server_id, input);
        self.process_manager.write_stdin(server_id, input).await
    }

    fn get_stdout_stream(
        &self,
        server_id: &str,
    ) -> Result<mpsc::Receiver<String>, ServerManagerError> {
        debug!(log_type = "server_manager", "Getting stdout stream for server: {}", server_id);
        self.process_manager.get_stdout_stream(server_id)
    }

    fn is_process_running(&self, server_id: &str) -> Result<bool, ServerManagerError> {
        let result = self.process_manager.is_process_running(server_id)?;
        debug!(log_type = "server_manager", "Checking if server {} is running: {}", server_id, result);
        Ok(result)
    }

    async fn stop_process(&self, server_id: &str) -> Result<(), ServerManagerError> {
        debug!(log_type = "server_manager", "Stopping process for server: {}", server_id);
        let result = self.process_manager.stop_process(server_id).await?;
        debug!(log_type = "server_manager", "Process for server {} stopped successfully", server_id);
        Ok(result)
    }
}

impl Default for LocalProvider {
    fn default() -> Self {
        Self::new()
    }
}
