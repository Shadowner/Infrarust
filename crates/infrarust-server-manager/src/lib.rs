//! Server manager for Infrarust.
//!
//! Provides automatic server start/stop with support for Local (Java process),
//! Pterodactyl (panel API), and Crafty Controller (panel API).
//!
//! The `ServerProvider` trait is non-sealed and can be implemented by third parties
//! for custom panel integrations.

pub mod crafty;
pub mod error;
pub mod local;
mod monitor;
pub mod provider;
pub mod pterodactyl;
pub mod service;
pub mod state;

pub use crafty::CraftyProvider;
pub use error::ServerManagerError;
pub use local::LocalProvider;
pub use provider::{ProviderStatus, ServerProvider};
pub use pterodactyl::PterodactylProvider;
pub use service::{PlayerCounter, ServerManagerService};
pub use state::ServerState;
