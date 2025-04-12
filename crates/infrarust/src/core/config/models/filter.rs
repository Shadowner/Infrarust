use serde::Deserialize;
use uuid::Uuid;

use super::{access_list::AccessListConfig, ban::BanConfig};
use crate::security::filter::RateLimiterConfig;

#[derive(Debug, Clone, Deserialize)]
pub struct FilterConfig {
    pub rate_limiter: Option<RateLimiterConfig>,
    pub ip_filter: Option<AccessListConfig<String>>,
    pub id_filter: Option<AccessListConfig<Uuid>>,
    pub name_filter: Option<AccessListConfig<String>>,
    #[serde(default)]
    pub ban: BanConfig,
}
