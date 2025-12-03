use crate::ServerState;
use crate::api::{ApiProvider, ApiServerStatus};
use crate::error::ServerManagerError;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct CraftyServerInfo {
    pub server_id: String,
    pub created: String,
    pub server_name: String,
    pub path: String,
    pub executable: String,
    pub log_path: String,
    pub execution_command: String,
    pub auto_start: bool,
    pub auto_start_delay: u64,
    pub crash_detection: bool,
    pub stop_command: String,
    pub executable_update_url: String,
    pub server_ip: String,
    pub server_port: u64,
    pub logs_delete_after: u64,

    #[serde(rename = "type")]
    pub server_type: String,

    pub show_status: bool,
    pub created_by: u64,
    pub shutdown_timeout: u64,
    pub ignored_exits: String,
    pub count_players: bool,
}

struct MinimalServerInfo {
    pub server_id: String,
    pub server_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CraftyServerResponse {
    pub status: String,
    pub data: CraftyServerInfo,
}

pub struct CraftyClient {
    client: Client,
    api_key: String,
    base_url: String,
}

impl CraftyClient {
    pub fn new(api_key: String, base_url: String) -> Self {
        let client = Client::new();
        CraftyClient {
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
        let url = format!("{}/api/v2/servers/{}", self.base_url, server_id);

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

        let info: CraftyServerResponse = response.json().await.map_err(|e| {
            ServerManagerError::ApiError(format!("Failed to parse response: {}", e))
        })?;

        Ok(MinimalServerInfo {
            server_id: info.data.server_id,
            server_name: info.data.server_name,
        })
    }
}

#[async_trait]
impl ApiProvider for CraftyClient {
    async fn get_server_status(
        &self,
        server_id: &str,
    ) -> Result<ApiServerStatus, ServerManagerError> {
        Ok(())
    }

    async fn start_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        Ok(())
    }
}
