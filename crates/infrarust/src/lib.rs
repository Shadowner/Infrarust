//! Infrarust - A Minecraft proxy server implementation in Rust
//!
//! This crate provides a flexible and efficient proxy server for Minecraft,
//! supporting multiple backend servers, protocol versions, and various security features.
//! It's designed to proxy multiple domain names to different type of Minecraft servers

// Core modules
pub mod core;
pub use core::error::{InfrarustError, Result as InfrarustResult, RsaError};

pub use infrarust_config::InfrarustConfig;

pub mod telemetry;

// Network and security modules
pub mod network;
pub mod security;
pub use network::proxy_protocol::reader::ProxyProtocolReader;
pub use network::{
    connection::{Connection, ServerConnection},
    proxy_protocol::write_proxy_protocol_header,
};
pub mod proxy_modes;
pub use security::{
    encryption::EncryptionState,
    filter::{Filter, FilterConfig, FilterError, FilterRegistry, FilterType},
    rate_limiter::RateLimiter,
};

// Server implementation
pub mod cli;
pub mod server;

mod infrarust;

use std::sync::Arc;

use crate::core::shared_component::SharedComponent;
use crate::server::gateway::Gateway;

#[derive(Debug)]
pub struct Infrarust {
    shared: Arc<SharedComponent>,
    gateway: Arc<Gateway>,
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore = "TODO"]
    async fn test_infrared_basic() {
        // TODO: Add integration tests that simulate client connections
    }
}
