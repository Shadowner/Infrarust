use infrarust_api::services::ban_service::BanEntry;
use serde::Serialize;

use crate::util::{ban_target_type_str, format_duration, format_system_time};

#[derive(Serialize)]
pub struct BanResponse {
    pub target_type: String,
    pub target_value: String,
    pub reason: Option<String>,
    pub expires_at: Option<String>,
    pub expires_in: Option<String>,
    pub created_at: String,
    pub source: String,
    pub permanent: bool,
}

impl BanResponse {
    pub fn from_entry(entry: &BanEntry) -> Self {
        let target_type = ban_target_type_str(&entry.target).to_string();
        let target_value = match &entry.target {
            infrarust_api::services::ban_service::BanTarget::Ip(ip) => ip.to_string(),
            infrarust_api::services::ban_service::BanTarget::Username(name) => name.clone(),
            infrarust_api::services::ban_service::BanTarget::Uuid(uuid) => uuid.to_string(),
            other => {
                tracing::warn!(?other, "Unknown BanTarget variant");
                "unknown".to_string()
            }
        };

        Self {
            target_type,
            target_value,
            reason: entry.reason.clone(),
            expires_at: entry.expires_at.map(format_system_time),
            expires_in: entry.remaining().map(format_duration),
            created_at: format_system_time(entry.created_at),
            source: entry.source.clone(),
            permanent: entry.is_permanent(),
        }
    }
}

#[derive(Serialize)]
pub struct BanCheckResponse {
    pub banned: bool,
    pub ban: Option<BanResponse>,
}
