use std::time::Duration;

use infrarust_config::PterodactylManagerConfig;

use crate::error::ServerManagerError;
use crate::provider::{ProviderStatus, ServerProvider};

/// Provider for Pterodactyl panel servers via REST API.
pub struct PterodactylProvider {
    http_client: reqwest::Client,
    api_url: String,
    api_key: String,
    server_id: String,
}

impl PterodactylProvider {
    pub fn new(config: &PterodactylManagerConfig, http_client: reqwest::Client) -> Self {
        Self {
            http_client,
            api_url: config.api_url.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
            server_id: config.server_id.clone(),
        }
    }

    /// Sends a power signal to the server.
    async fn send_power_signal(&self, signal: &str) -> Result<(), ServerManagerError> {
        let url = format!(
            "{}/api/client/servers/{}/power",
            self.api_url, self.server_id
        );

        let resp = self
            .http_client
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("Accept", "application/json")
            .json(&serde_json::json!({"signal": signal}))
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ServerManagerError::ApiResponse(format!(
                "Pterodactyl power {signal} returned {status}: {body}"
            )));
        }

        Ok(())
    }
}

impl ServerProvider for PterodactylProvider {
    fn start(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), ServerManagerError>> + Send + '_>,
    > {
        Box::pin(async move {
            tracing::info!(server_id = %self.server_id, "sending start signal to Pterodactyl");
            self.send_power_signal("start").await
        })
    }

    fn stop(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), ServerManagerError>> + Send + '_>,
    > {
        Box::pin(async move {
            tracing::info!(server_id = %self.server_id, "sending stop signal to Pterodactyl");
            self.send_power_signal("stop").await
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
            let url = format!(
                "{}/api/client/servers/{}/resources",
                self.api_url, self.server_id
            );

            let resp = self
                .http_client
                .get(&url)
                .bearer_auth(&self.api_key)
                .header("Accept", "application/json")
                .timeout(Duration::from_secs(10))
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(ServerManagerError::ApiResponse(format!(
                    "Pterodactyl resources returned {status}: {body}"
                )));
            }

            let body: serde_json::Value = resp.json().await?;
            let Some(current_state) = body["attributes"]["current_state"].as_str() else {
                tracing::warn!(
                    server_id = %self.server_id,
                    "failed to extract current_state from Pterodactyl response, defaulting to Unknown"
                );
                return Ok(ProviderStatus::Unknown);
            };

            Ok(map_pterodactyl_state(current_state))
        })
    }

    fn provider_type(&self) -> &'static str {
        "pterodactyl"
    }
}

fn map_pterodactyl_state(state: &str) -> ProviderStatus {
    match state {
        "running" => ProviderStatus::Running,
        "starting" => ProviderStatus::Starting,
        "stopping" => ProviderStatus::Stopping,
        "offline" => ProviderStatus::Stopped,
        _ => ProviderStatus::Unknown,
    }
}
