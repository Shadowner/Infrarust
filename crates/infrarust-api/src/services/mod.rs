//! Proxy service traits.
//!
//! All service traits are **sealed** — only the proxy implements them.
//! Plugins access services through the [`PluginContext`](crate::plugin::PluginContext).

pub mod ban_service;
pub mod config_service;
pub mod player_registry;
pub mod scheduler;
pub mod server_manager;

pub use ban_service::{BanEntry, BanService, BanTarget};
pub use config_service::{ConfigService, ProxyMode, ServerConfig};
pub use player_registry::PlayerRegistry;
pub use scheduler::{Scheduler, TaskHandle};
pub use server_manager::{ServerManager, ServerState};
