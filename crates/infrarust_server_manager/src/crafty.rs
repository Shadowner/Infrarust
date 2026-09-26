use infrarust_config::CraftyManagerConfig;

use crate::error::ServerManagerError;
use crate::http::{HTTP_TIMEOUT, check_response};
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

    async fn send_action(&self, action: &str) -> Result<(), ServerManagerError> {
        tracing::info!(server_id = %self.server_id, action, "sending action to Crafty");

        let url = format!(
            "{}/api/v2/servers/{}/action/{action}",
            self.api_url, self.server_id
        );

        let resp = self
            .http_client
            .post(&url)
            .bearer_auth(&self.api_key)
            .timeout(HTTP_TIMEOUT)
            .send()
            .await?;

        check_response(resp, &format!("Crafty {action}")).await?;
        Ok(())
    }
}

impl ServerProvider for CraftyProvider {
    fn start(
        &self,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>> {
        Box::pin(self.send_action("start_server"))
    }

    fn stop(
        &self,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>> {
        Box::pin(self.send_action("stop_server"))
    }

    fn check_status(
        &self,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<ProviderStatus, ServerManagerError>> + Send + '_>,
    > {
        Box::pin(async move {
            let url = format!("{}/api/v2/servers/{}/stats", self.api_url, self.server_id);

            let resp = self
                .http_client
                .get(&url)
                .bearer_auth(&self.api_key)
                .timeout(HTTP_TIMEOUT)
                .send()
                .await?;

            let body: serde_json::Value =
                check_response(resp, "Crafty stats").await?.json().await?;
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
