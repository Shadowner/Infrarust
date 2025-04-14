use serde::Deserialize;
use uuid::Uuid;

use super::{access_list::AccessListConfig, ban::BanConfig};

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimiterConfig {
    pub enabled: bool,
    pub requests_per_minute: u32,
    pub burst_size: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilterConfig {
    pub rate_limiter: Option<RateLimiterConfig>,
    pub ip_filter: Option<AccessListConfig<String>>,
    pub id_filter: Option<AccessListConfig<Uuid>>,
    pub name_filter: Option<AccessListConfig<String>>,
    #[serde(default)]
    pub ban: BanConfig,
}
