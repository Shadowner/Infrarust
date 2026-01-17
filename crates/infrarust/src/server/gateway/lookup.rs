use std::sync::Arc;

use infrarust_config::{ServerConfig, models::logging::LogType};
use tracing::{debug, instrument};

use super::Gateway;

impl Gateway {
    #[instrument(skip(self), fields(domain = %domain), level = "debug")]
    pub(crate) async fn find_server(&self, domain: &str) -> Option<Arc<ServerConfig>> {
        debug!(
            log_type = LogType::Authentication.as_str(),
            "Finding server by domain: {}", domain
        );
        let configs = self
            .shared
            .configuration_service()
            .get_all_configurations()
            .await;
        debug!(
            log_type = LogType::Authentication.as_str(),
            "Got {} total server configurations",
            configs.len()
        );

        let result = self
            .shared
            .configuration_service()
            .find_server_by_domain(domain)
            .await;

        debug!(
            domain = %domain,
            found = result.is_some(),
            "Domain lookup result"
        );

        if result.is_some() {
            debug!(
                log_type = LogType::Authentication.as_str(),
                "Found server for domain {}", domain
            );
        } else {
            debug!(
                log_type = LogType::Authentication.as_str(),
                "No server found for domain {}", domain
            );
        }

        result
    }

    pub async fn get_server_from_ip(&self, ip: &str) -> Option<Arc<ServerConfig>> {
        self.shared
            .configuration_service()
            .find_server_by_ip(ip)
            .await
    }
}
