use crate::api::{ApiProvider, ApiServerStatus};
use crate::error::ServerManagerError;
use crate::ServerState;
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
struct ServerLimits {
    pub memory: u64,
    pub swap: u64,
    pub disk: u64,
    pub io: u64,
    pub cpu: u64,
    pub threads: Option<u64>,
    pub oom_disabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct FeatureLimits {
    pub databases: u64,
    pub allocations: u64,
    pub backups: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct AllocationAttributes {
    pub id: u64,
    pub ip: String,
    pub ip_alias: String,
    pub port: u64,
    pub notes: Option<String>,
    pub is_default: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Allocation {
    pub object: String,
    pub attributes: AllocationAttributes,
}

#[derive(Debug, Serialize, Deserialize)]
struct Allocations {
    pub object: String,
    pub data: Vec<Allocation>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Relationships {
    pub allocations: Allocations,
}

#[derive(Debug, Serialize, Deserialize)]
struct ServerInfoAttributes {
    pub server_owner: bool,
    pub identifier: String,
    pub internal_id: u64,
    pub uuid: String,
    pub name: String,
    pub node: String,
    pub is_node_under_maintenance: bool,
    pub description: String,
    pub limits: ServerLimits,
    pub feature_limits: FeatureLimits,
    pub status: Option<String>,
    pub is_suspended: bool,
    pub is_installing: bool,
    pub is_transferring: bool,
    pub relationships: Relationships,
}

#[derive(Debug, Serialize, Deserialize)]
struct PterodactylServerInfo {
    pub object: String,
    pub attributes: ServerInfoAttributes,
}

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
    ) -> Result<ServerInfoAttributes, ServerManagerError> {
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

        let info: PterodactylServerInfo = response.json().await.map_err(|e| {
            ServerManagerError::ApiError(format!("Failed to parse response: {}", e))
        })?;

        Ok(info.attributes)
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
            name: server_info.name,
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
