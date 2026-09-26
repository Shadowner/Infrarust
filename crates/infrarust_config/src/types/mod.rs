//! Fundamental types: enums, value objects, and shared configuration structs.

mod address;
mod auth;
pub(crate) mod balance;
mod ban;
mod docker;
mod events;
mod forwarding;
mod health;
mod ip_filter;
mod network;
mod permissions;
mod plugin_messaging;
mod proxy_mode;
mod rate_limit;
mod server_manager;
mod status;
mod telemetry;
mod wasm;
mod wasm_network;
mod web;

pub use address::{DomainRewrite, ServerAddress};
pub use auth::{AuthConfig, OfflineUuidPolicy};
pub use balance::{BalanceConfig, BalanceStrategy, WeightedAddress};
pub use ban::{BanConfig, BanProviderSelection};
pub use docker::DockerProviderConfig;
pub use events::EventsConfig;
pub use forwarding::{BungeeCordChannelPermissions, ForwardingConfig, ForwardingMode};
pub use health::{ActiveHealthConfig, ProbeKind};
pub use ip_filter::IpFilterConfig;
pub use network::{KeepaliveConfig, TimeoutConfig};
pub use permissions::{PermissionProviderSelection, PermissionsConfig};
pub use plugin_messaging::PluginMessagingConfig;
pub use proxy_mode::ProxyMode;
pub use rate_limit::RateLimitConfig;
pub use server_manager::{
    CraftyManagerConfig, LocalManagerConfig, PterodactylManagerConfig, ServerManagerConfig,
};
pub use status::{MotdConfig, MotdEntry, StatusCacheConfig};
pub use telemetry::{MetricsConfig, ResourceConfig, TelemetryConfig, TracesConfig};
pub use wasm::{
    PluginWasmConfig, PluginWasmRecoveryConfig, WasmConfig, WasmLimits, WasmRecoveryConfig,
};
pub use wasm_network::{
    HostPattern, NetworkRule, NetworkRuleError, PortRange, WasmMount, WasmNetworkConfig,
};
pub use web::WebConfig;

/// Default Minecraft port.
pub const DEFAULT_MC_PORT: u16 = 25565;
