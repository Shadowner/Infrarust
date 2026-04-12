//! Status ping handling: relay, cache, favicon, and response construction.
//!
//! Handles modern (1.7+) status pings. Legacy status is handled by
//! `handler::legacy`.

use infrarust_protocol::version::ProtocolVersion;

/// Fixed protocol version used for all Status-state packet encode/decode operations.
/// Using a fixed known version for registry lookups avoids decode failures when the
/// connecting client reports a protocol version not yet in [`ProtocolVersion::SUPPORTED`].
pub const STATUS_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V1_7_2;

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
