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
}

/// Provider for managing local server processes
#[derive(Debug, Clone)]
pub struct LocalProvider {
    process_manager: Arc<ProcessManager>,
    configs: Arc<Mutex<HashMap<String, LocalServerConfig>>>,
}

impl LocalProvider {
    pub fn new() -> Self {
        debug!("Creating new LocalProvider instance");
        Self {
            process_manager: Arc::new(ProcessManager::new()),
            configs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register_server(&self, server_id: &str, config: LocalServerConfig) {
        debug!("Registering server with id: {}", server_id);
        let mut configs = self.configs.lock().unwrap();
        configs.insert(server_id.to_string(), config);
    }

    pub fn unregister_server(&self, server_id: &str) {
        debug!("Unregistering server with id: {}", server_id);
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
        debug!("Getting status for server: {}", server_id);
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
            ServerState::Running
        } else {
            ServerState::Stopped
        };

        debug!("Server {} status: {:?}", server_id, state);
        Ok(ApiServerStatus {
            id: server_id.to_string(),
            name,
            status: state.clone(),
            is_running,
            is_crashed: false, // Not supported yet
            error: None,
        })
    }

    async fn start_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        debug!("Attempting to start server: {}", server_id);
        // Check if already running
        if self.process_manager.is_process_running(server_id)? {
            debug!("Server {} is already running", server_id);
            return Ok(());
        }

        // Get server config
        let config = {
            let configs = self.configs.lock().unwrap();
            match configs.get(server_id) {
                Some(config) => config.clone(),
                None => {
                    debug!("No configuration found for server {}", server_id);
                    return Err(ServerManagerError::ProcessError(format!(
                        "No configuration found for server {}",
                        server_id
                    )));
                }
            }
        };

        // Convert args to a slice of &str
        let args: Vec<&str> = config.args.iter().map(|s| s.as_str()).collect();

        debug!("Starting server {} with executable: {}", server_id, config.executable);
        // Start the process
        let _ = self
            .process_manager
            .start_process(server_id, &config.executable, &args)?;

        debug!("Server {} started successfully", server_id);
        Ok(())
    }

    async fn stop_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        debug!("Attempting to stop server: {}", server_id);
        // First try to send a graceful shutdown command (like "stop" or "exit")
        // This may not work for all server types, so we'll also force stop if needed
        debug!("Sending graceful shutdown commands to server: {}", server_id);
        let _ = self.process_manager.write_stdin(server_id, "stop").await;
        let _ = self.process_manager.write_stdin(server_id, "exit").await;

        // Give it a moment to shut down gracefully
        debug!("Waiting for server {} to shut down gracefully", server_id);
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // If still running, force stop it
        if self.process_manager.is_process_running(server_id)? {
            debug!("Server {} still running after graceful shutdown attempt, forcing stop", server_id);
            self.process_manager.stop_process(server_id).await?;
        }

        debug!("Server {} stopped successfully", server_id);
        Ok(())
    }
}

#[async_trait]
impl ProcessProvider for LocalProvider {
    async fn write_stdin(&self, server_id: &str, input: &str) -> Result<(), ServerManagerError> {
        debug!("Writing to stdin for server {}: '{}'", server_id, input);
        self.process_manager.write_stdin(server_id, input).await
    }

    fn get_stdout_stream(
        &self,
        server_id: &str,
    ) -> Result<mpsc::Receiver<String>, ServerManagerError> {
        debug!("Getting stdout stream for server: {}", server_id);
        self.process_manager.get_stdout_stream(server_id)
    }

    fn is_process_running(&self, server_id: &str) -> Result<bool, ServerManagerError> {
        let result = self.process_manager.is_process_running(server_id)?;
        debug!("Checking if server {} is running: {}", server_id, result);
        Ok(result)
    }

    async fn stop_process(&self, server_id: &str) -> Result<(), ServerManagerError> {
        debug!("Stopping process for server: {}", server_id);
        let result = self.process_manager.stop_process(server_id).await?;
        debug!("Process for server {} stopped successfully", server_id);
        Ok(result)
    }
}

impl Default for LocalProvider {
    fn default() -> Self {
        Self::new()
    }
}
