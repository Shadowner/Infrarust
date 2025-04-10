use crate::ServerState;
use crate::api::{ApiProvider, ApiServerStatus};
use crate::error::ServerManagerError;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

pub struct ApiClient {
    client: Client,
    base_url: String,
    auth_token: Option<String>,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        let client = Client::new();
        ApiClient {
            client,
            base_url: base_url.to_string(),
            auth_token: None,
        }
    }

    pub fn with_auth(base_url: &str, auth_token: &str) -> Self {
        let client = Client::new();
        ApiClient {
            client,
            base_url: base_url.to_string(),
            auth_token: Some(auth_token.to_string()),
        }
    }
}

#[derive(Deserialize)]
struct ApiResponseStatus {
    id: String,
    name: String,
    status: String,
    is_running: bool,
    is_crashed: bool,
    error: Option<String>,
}

#[async_trait]
impl ApiProvider for ApiClient {
    async fn get_server_status(
        &self,
        server_id: &str,
    ) -> Result<ApiServerStatus, ServerManagerError> {
        let url = format!("{}/servers/{}/status", self.base_url, server_id);

        let mut request = self.client.get(&url);
        if let Some(token) = &self.auth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request.send().await.map_err(|e| {
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

        let status: ApiResponseStatus = response.json().await.map_err(|e| {
            ServerManagerError::ApiError(format!("Failed to parse response: {}", e))
        })?;

        Ok(ApiServerStatus {
            id: status.id,
            name: status.name,
            status: ServerState::from(status.status.as_str()),
            is_running: status.is_running,
            is_crashed: status.is_crashed,
            error: status.error,
        })
    }

    async fn start_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        let url = format!("{}/servers/{}/start", self.base_url, server_id);

        let mut request = self.client.post(&url);
        if let Some(token) = &self.auth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request
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
        let url = format!("{}/servers/{}/stop", self.base_url, server_id);

        let mut request = self.client.post(&url);
        if let Some(token) = &self.auth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request
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
}
