//! Proxy service traits.
//!
//! All service traits are **sealed** — only the proxy implements them.
//! Plugins access services through the [`PluginContext`](crate::plugin::PluginContext).

pub mod ban_service;
pub mod caller_deadline;
pub mod config_service;
pub mod load_balancer;
pub mod player_registry;
pub mod plugin_registry;
pub mod providers;
pub mod proxy_info;
pub mod scheduler;
pub mod server_manager;
pub mod service_registry;

pub use ban_service::{
    BanEntry, BanFeatures, BanPage, BanProvider, BanQuery, BanRequest, BanService, BanSource,
    BanTarget, BanVerdict, IpNet, LoginAttempt, LoginStage, UnbanRequest,
};
pub use config_service::{ConfigService, ConfigWriteError, ProxyMode, ServerConfig, ServerSource};
pub use load_balancer::{BackendState, BackendStatus, LbError, LoadBalancerService};
pub use player_registry::PlayerRegistry;
pub use plugin_registry::{PluginInfo, PluginRegistry};
pub use providers::{ProviderKind, ProviderRejected};
pub use proxy_info::{
    KeepaliveInfo, ProxyInfo, RateLimitInfo, StatusCacheInfo, UnknownDomainBehavior,
};
pub use scheduler::{AsyncTask, RepeatingTask, Scheduler, TaskHandle};
pub use server_manager::{ServerManager, ServerState};
pub use service_registry::{ServiceHandle, ServiceRegistry, ServiceRegistryExt};
