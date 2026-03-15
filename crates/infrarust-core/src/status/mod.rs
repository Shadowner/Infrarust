//! Status ping handling: relay, cache, favicon, and response construction.
//!
//! Handles modern (1.7+) status pings. Legacy status is handled by
//! `handler::legacy`.

pub mod cache;
pub mod favicon;
pub mod handler;
pub mod relay;
pub mod response;

pub use cache::StatusCache;
pub use favicon::{FaviconCache, load_favicon};
pub use handler::StatusHandler;
pub use relay::StatusRelayClient;
pub use response::ServerPingResponse;
