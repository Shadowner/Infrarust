use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::api::{ApiProvider, ApiServerStatus};
use crate::error::ServerManagerError;
use crate::process::ProcessProvider;
use crate::{ProcessManager, ServerState};

/// Configuration for a local server
#[derive(Clone, Debug)]
pub struct LocalServerConfig {
    /// The executable to run
    pub executable: String,
    /// Arguments to pass to the executable
    pub args: Vec<String>,
    /// Working directory for the server
    pub working_dir: Option<String>,
}

/// Provider for managing local server processes
#[derive(Clone)]
pub struct LocalProvider {
    process_manager: Arc<ProcessManager>,
    configs: Arc<Mutex<HashMap<String, LocalServerConfig>>>,
}

impl LocalProvider {
    pub fn new() -> Self {
        Self {
            process_manager: Arc::new(ProcessManager::new()),
            configs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register_server(&self, server_id: &str, config: LocalServerConfig) {
        let mut configs = self.configs.lock().unwrap();
        configs.insert(server_id.to_string(), config);
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
        // Check if already running
        if self.process_manager.is_process_running(server_id)? {
            return Ok(());
        }

        // Get server config
        let config = {
            let configs = self.configs.lock().unwrap();
            match configs.get(server_id) {
                Some(config) => config.clone(),
                None => {
                    return Err(ServerManagerError::ProcessError(format!(
                        "No configuration found for server {}",
                        server_id
                    )));
                }
            }
        };

        // Convert args to a slice of &str
        let args: Vec<&str> = config.args.iter().map(|s| s.as_str()).collect();

        // Start the process
        let _ = self
            .process_manager
            .start_process(server_id, &config.executable, &args)?;

        Ok(())
    }

    async fn stop_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        // First try to send a graceful shutdown command (like "stop" or "exit")
        // This may not work for all server types, so we'll also force stop if needed
        let _ = self.process_manager.write_stdin(server_id, "stop").await;
        let _ = self.process_manager.write_stdin(server_id, "exit").await;

        // Give it a moment to shut down gracefully
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // If still running, force stop it
        if self.process_manager.is_process_running(server_id)? {
            self.process_manager.stop_process(server_id).await?;
        }

        Ok(())
    }
}

#[async_trait]
impl ProcessProvider for LocalProvider {
    async fn write_stdin(&self, server_id: &str, input: &str) -> Result<(), ServerManagerError> {
        self.process_manager.write_stdin(server_id, input).await
    }

    fn get_stdout_stream(
        &self,
        server_id: &str,
    ) -> Result<mpsc::Receiver<String>, ServerManagerError> {
        self.process_manager.get_stdout_stream(server_id)
    }

    fn is_process_running(&self, server_id: &str) -> Result<bool, ServerManagerError> {
        self.process_manager.is_process_running(server_id)
    }

    async fn stop_process(&self, server_id: &str) -> Result<(), ServerManagerError> {
        self.process_manager.stop_process(server_id).await
    }
}

impl Default for LocalProvider {
    fn default() -> Self {
        Self::new()
    }
}
