use crate::ServerState;
use crate::api::{ApiProvider, ApiServerStatus};
use crate::error::ServerManagerError;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct PterodactylResources {
    pub memory_bytes: u64,
    pub cpu_absolute: f64,
    pub disk_bytes: u64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct PterodactylStatusAttributes {
    pub current_state: String,
    pub is_suspended: bool,
    pub resources: PterodactylResources,
}

#[derive(Debug, Serialize, Deserialize)]
struct PterodactylStatus {
    pub object: String,
    pub attributes: PterodactylStatusAttributes,
}

#[derive(Debug, Serialize, Deserialize)]
struct MinimalAttributes {
    pub name: String,
    pub description: String,
    pub identifier: String,
}
#[derive(Debug, Serialize, Deserialize)]
struct MinimalServerInfo {
    pub object: String,
    pub attributes: MinimalAttributes,
}
#[derive(Debug, Clone)]
pub struct PterodactylClient {
    client: Client,
    api_key: String,
    base_url: String,
}

impl PterodactylClient {
    pub fn new(api_key: String, base_url: String) -> Self {
        let client = Client::new();
        PterodactylClient {
            client,
            api_key,
            base_url,
        }
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.api_key)
    }

    async fn get_server_info(
        &self,
        server_id: &str,
    ) -> Result<MinimalServerInfo, ServerManagerError> {
        let url = format!("{}/api/client/servers/{}", self.base_url, server_id);

        let response = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| {
                ServerManagerError::ApiError(format!("Failed to get server info: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(ServerManagerError::ApiError(format!(
                "API error ({}): {}",
                status, text
            )));
        }

        let info: MinimalServerInfo = response.json().await.map_err(|e| {
            ServerManagerError::ApiError(format!("Failed to parse response: {}", e))
        })?;

        Ok(info)
    }
}

#[async_trait]
impl ApiProvider for PterodactylClient {
    async fn get_server_status(
        &self,
        server_id: &str,
    ) -> Result<ApiServerStatus, ServerManagerError> {
        let url = format!(
            "{}/api/client/servers/{}/resources",
            self.base_url, server_id
        );

        let server_info = self.get_server_info(server_id).await?;

        let response = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| {
                ServerManagerError::ApiError(format!("Failed to get server status: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(ServerManagerError::ApiError(format!(
                "API error ({}): {}",
                status, text
            )));
        }

        let status: PterodactylStatus = response.json().await.map_err(|e| {
            ServerManagerError::ApiError(format!("Failed to parse response: {}", e))
        })?;

        let is_running = status.attributes.current_state == "running";
        let is_crashed = status.attributes.current_state == "crashed";
        let error = if status.attributes.is_suspended {
            Some("Server is suspended".to_string())
        } else if status.attributes.current_state == "crashed" {
            Some("Server has crashed".to_string())
        } else {
            None
        };

        Ok(ApiServerStatus {
            id: server_id.to_string(),
            name: server_info.attributes.name,
            status: ServerState::from(status.attributes.current_state.as_str()),
            is_running,
            is_crashed,
            error,
        })
    }

    async fn start_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        let url = format!("{}/api/client/servers/{}/power", self.base_url, server_id);

        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"signal": "start"}))
            .send()
            .await
            .map_err(|e| ServerManagerError::ApiError(format!("Failed to start server: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(ServerManagerError::ApiError(format!(
                "API error ({}): {}",
                status, text
            )));
        }

        Ok(())
    }

    async fn stop_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        let url = format!("{}/api/client/servers/{}/power", self.base_url, server_id);

        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"signal": "stop"}))
            .send()
            .await
            .map_err(|e| ServerManagerError::ApiError(format!("Failed to stop server: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(ServerManagerError::ApiError(format!(
                "API error ({}): {}",
                status, text
            )));
        }

        Ok(())
    }

    async fn restart_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        let url = format!("{}/api/client/servers/{}/power", self.base_url, server_id);

        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"signal": "restart"}))
            .send()
            .await
            .map_err(|e| {
                ServerManagerError::ApiError(format!("Failed to restart server: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(ServerManagerError::ApiError(format!(
                "API error ({}): {}",
                status, text
            )));
        }

        Ok(())
    }
}
