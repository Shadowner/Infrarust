use std::fmt::Debug;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::ServerManagerError;

/// Trait for providers that can interact with server processes,
/// providing real-time stdout and stdin capabilities
#[async_trait]
pub trait ProcessProvider: Send + Sync + Debug {
    async fn write_stdin(&self, server_id: &str, input: &str) -> Result<(), ServerManagerError>;
    fn get_stdout_stream(
        &self,
        server_id: &str,
    ) -> Result<mpsc::Receiver<String>, ServerManagerError>;
    fn is_process_running(&self, server_id: &str) -> Result<bool, ServerManagerError>;
    async fn stop_process(&self, server_id: &str) -> Result<(), ServerManagerError>;
}
