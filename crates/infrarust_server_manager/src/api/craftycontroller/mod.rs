use crate::ServerState;
use crate::api::{ApiProvider, ApiServerStatus};
use crate::error::ServerManagerError;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::error;

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

#[derive(Debug, Serialize, Deserialize)]
struct CraftyServerStatsResponse {
    pub status: String,
    pub data: CraftyServerStats,
}

#[derive(Debug, Serialize, Deserialize)]
struct CraftyServerStats {
    pub server_id: CraftyServerInfo,

    // pub stats_id: u64,
    // pub server_port: u64,
    // pub online: u64,
    // pub max: u64,

    // pub mem_percent: f64,
    // pub mem: f64,
    // pub cpu: f64,

    // pub created: String,
    // pub started: String,
    // pub mem: String,
    // pub world_name: String,
    // pub world_size: String,
    // pub int_ping_results: String,
    // pub players: String,
    // pub desc: String,
    // pub icon: Option<String>,
    // pub version: String,
    pub running: bool,
    pub updating: bool,
    pub waiting_start: bool,
    pub first_run: bool,
    pub crashed: bool,
    pub importing: bool,
}
#[derive(Debug, Clone)]
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

    ////// Crafty returns minimal server info in the status response, so this is unused.
    // async fn get_server_info(
    //     &self,
    //     server_id: &str,
    // ) -> Result<MinimalServerInfo, ServerManagerError> {
    //     let url = format!("{}/api/v2/servers/{}", self.base_url, server_id);

    //     let response = self
    //         .client
    //         .get(&url)
    //         .header("Authorization", self.auth_header())
    //         .header("Accept", "application/json")
    //         .header("Content-Type", "application/json")
    //         .send()
    //         .await
    //         .map_err(|e| {
    //             ServerManagerError::ApiError(format!("Failed to get server info: {}", e))
    //         })?;

    //     if !response.status().is_success() {
    //         let status = response.status();
    //         let text = response
    //             .text()
    //             .await
    //             .unwrap_or_else(|_| "Unknown error".to_string());
    //         return Err(ServerManagerError::ApiError(format!(
    //             "API error ({}): {}",
    //             status, text
    //         )));
    //     }

    //     let info: CraftyServerResponse = response.json().await.map_err(|e| {
    //         ServerManagerError::ApiError(format!("Failed to parse response: {}", e))
    //     })?;

    //     Ok(MinimalServerInfo {
    //         server_id: info.data.server_id,
    //         server_name: info.data.server_name,
    //     })
    // }
    ///////
}

#[async_trait]
impl ApiProvider for CraftyClient {
    async fn get_server_status(
        &self,
        server_id: &str,
    ) -> Result<ApiServerStatus, ServerManagerError> {
        // Ok(())

        let url = format!("{}/api/v2/servers/{}/stats", self.base_url, server_id);

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
            error!("Bad status code on response: {}", response.status());
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

        let status: CraftyServerStatsResponse = response.json().await.map_err(|e| {
            ServerManagerError::ApiError(format!("Failed to parse response: {}", e))
        })?;

        let is_running = status.data.running;
        let is_crashed = status.data.crashed;
        let error = if !is_running && !is_crashed {
            Some("Server is offline".to_string())
        } else if is_crashed {
            Some("Server has crashed".to_string())
        } else {
            None
        };
        let server_state = if is_running {
            ServerState::Running
        } else if is_crashed {
            ServerState::Crashed
        } else {
            ServerState::Stopped
        };

        Ok(ApiServerStatus {
            id: server_id.to_string(),
            name: status.data.server_id.server_name,
            status: server_state,
            is_running,
            is_crashed,

            error,
        })
    }

    async fn start_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        let url = format!(
            "{}/api/v2/servers/{}/actions/start_server",
            self.base_url, server_id
        );
        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| {
                ServerManagerError::ApiError(format!("Failed to post start server action: {}", e))
            })?;

        if !response.status().is_success() {
            error!("Bad status code on response: {}", response.status());
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
        let url = format!(
            "{}/api/v2/servers/{}/actions/stop_server",
            self.base_url, server_id
        );
        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| {
                ServerManagerError::ApiError(format!("Failed to post stop server action: {}", e))
            })?;
        if !response.status().is_success() {
            error!("Bad status code on response: {}", response.status());
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
        let url = format!(
            "{}/api/v2/servers/{}/actions/restart_server",
            self.base_url, server_id
        );
        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| {
                ServerManagerError::ApiError(format!("Failed to post restart server action: {}", e))
            })?;

        if !response.status().is_success() {
            error!("Bad status code on response: {}", response.status());
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
