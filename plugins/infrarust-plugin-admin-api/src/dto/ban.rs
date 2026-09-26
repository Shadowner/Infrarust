use infrarust_api::services::ban_service::BanEntry;
use serde::Serialize;

use crate::error::ApiError;
use crate::util::{BanTargetParts, format_duration, format_system_time};

#[derive(Serialize)]
pub struct BanResponse {
    pub id: String,
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
    pub fn from_entry(entry: &BanEntry) -> Result<Self, ApiError> {
        let target = BanTargetParts::try_from(&entry.target)?;
        Ok(Self {
            id: entry.id.clone(),
            target_type: target.kind.to_string(),
            target_value: target.value,
            reason: entry.reason.clone(),
            expires_at: entry.expires_at.map(format_system_time),
            expires_in: entry.remaining().map(format_duration),
            created_at: format_system_time(entry.created_at),
            source: entry.source.to_string(),
            permanent: entry.is_permanent(),
        })
    }
}

#[derive(Serialize)]
pub struct BanCheckResponse {
    pub banned: bool,
    pub ban: Option<BanResponse>,
}
