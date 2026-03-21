//! Configuration types for the Infrarust Minecraft proxy.

pub mod defaults;
pub mod domain;
pub mod error;
pub mod provider;
pub mod proxy;
pub mod server;
pub mod types;
pub mod validation;

// Main re-exports for ergonomics
pub use domain::DomainIndex;
pub use error::ConfigError;
pub use provider::{ConfigChange, ConfigProvider};
pub use proxy::{ProxyConfig, UnknownDomainBehavior};
pub use server::ServerConfig;
pub use types::*;
pub use validation::{validate_proxy_config, validate_server_config, validate_server_configs};
