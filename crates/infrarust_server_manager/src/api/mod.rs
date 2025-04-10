pub mod client;
pub mod mock;
pub mod models;
pub mod pterodactyl;

use crate::{error::ServerManagerError, ServerState};
use async_trait::async_trait;

#[async_trait]
pub trait ApiProvider: Send + Sync {
    async fn get_server_status(
        &self,
        server_id: &str,
    ) -> Result<ApiServerStatus, ServerManagerError>;

    async fn start_server(&self, server_id: &str) -> Result<(), ServerManagerError>;
    async fn stop_server(&self, server_id: &str) -> Result<(), ServerManagerError>;
    async fn restart_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        // Default implementation: stop then start
        self.stop_server(server_id).await?;
        self.start_server(server_id).await
    }
}

#[derive(Debug, Clone)]
pub struct ApiServerStatus {
    pub id: String,
    pub name: String,
    pub status: ServerState,
    pub is_running: bool,
    pub is_crashed: bool,
    pub error: Option<String>,
}

// Re-export useful items
pub use client::ApiClient;
pub use mock::MockApiProvider;
pub use models::*;
pub use pterodactyl::PterodactylClient;
