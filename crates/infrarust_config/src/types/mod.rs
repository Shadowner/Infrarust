//! Fundamental types: enums, value objects, and shared configuration structs.

mod address;
mod ban;
mod docker;
mod forwarding;
mod ip_filter;
mod network;
mod permissions;
mod proxy_mode;
mod rate_limit;
mod server_manager;
mod status;
mod telemetry;
mod web;

pub use address::{DomainRewrite, ServerAddress};
pub use ban::BanConfig;
pub use docker::DockerProviderConfig;
pub use forwarding::{BungeeCordChannelPermissions, ForwardingConfig, ForwardingMode};
pub use ip_filter::IpFilterConfig;
pub use network::{KeepaliveConfig, TimeoutConfig};
pub use permissions::PermissionsConfig;
pub use proxy_mode::ProxyMode;
pub use rate_limit::RateLimitConfig;
pub use server_manager::{
    CraftyManagerConfig, LocalManagerConfig, PterodactylManagerConfig, ServerManagerConfig,
};
pub use status::{MotdConfig, MotdEntry, StatusCacheConfig};
pub use telemetry::{MetricsConfig, ResourceConfig, TelemetryConfig, TracesConfig};
pub use web::WebConfig;

/// Default Minecraft port.
pub const DEFAULT_MC_PORT: u16 = 25565;
