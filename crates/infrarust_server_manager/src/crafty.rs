use std::time::Duration;

use infrarust_config::CraftyManagerConfig;

use crate::error::ServerManagerError;
use crate::provider::{ProviderStatus, ServerProvider};

/// Provider for Crafty Controller servers via REST API.
pub struct CraftyProvider {
    http_client: reqwest::Client,
    api_url: String,
    api_key: String,
    server_id: String,
}

impl CraftyProvider {
    pub fn new(config: &CraftyManagerConfig, http_client: reqwest::Client) -> Self {
        Self {
            http_client,
            api_url: config.api_url.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
            server_id: config.server_id.clone(),
        }
    }
}

impl ServerProvider for CraftyProvider {
    fn start(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), ServerManagerError>> + Send + '_>,
    > {
        Box::pin(async move {
            tracing::info!(server_id = %self.server_id, "sending start action to Crafty");

            let url = format!(
                "{}/api/v2/servers/{}/action/start_server",
                self.api_url, self.server_id
            );

            let resp = self
                .http_client
                .post(&url)
                .bearer_auth(&self.api_key)
                .timeout(Duration::from_secs(10))
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(ServerManagerError::ApiResponse(format!(
                    "Crafty start_server returned {status}: {body}"
                )));
            }

            Ok(())
        })
    }

    fn stop(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), ServerManagerError>> + Send + '_>,
    > {
        Box::pin(async move {
            tracing::info!(server_id = %self.server_id, "sending stop action to Crafty");

            let url = format!(
                "{}/api/v2/servers/{}/action/stop_server",
                self.api_url, self.server_id
            );

            let resp = self
                .http_client
                .post(&url)
                .bearer_auth(&self.api_key)
                .timeout(Duration::from_secs(10))
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(ServerManagerError::ApiResponse(format!(
                    "Crafty stop_server returned {status}: {body}"
                )));
            }

            Ok(())
        })
    }

    fn check_status(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ProviderStatus, ServerManagerError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            let url = format!("{}/api/v2/servers/{}/stats", self.api_url, self.server_id);

            let resp = self
                .http_client
                .get(&url)
                .bearer_auth(&self.api_key)
                .timeout(Duration::from_secs(10))
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(ServerManagerError::ApiResponse(format!(
                    "Crafty stats returned {status}: {body}"
                )));
            }

            let body: serde_json::Value = resp.json().await?;
            let Some(running) = body["data"]["running"].as_bool() else {
                tracing::warn!(
                    server_id = %self.server_id,
                    "failed to extract running status from Crafty response, defaulting to Unknown"
                );
                return Ok(ProviderStatus::Unknown);
            };

            Ok(if running {
                ProviderStatus::Running
            } else {
                ProviderStatus::Stopped
            })
        })
    }

    fn provider_type(&self) -> &'static str {
        "crafty"
    }
}
