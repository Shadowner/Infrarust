//! The frozen WIT contract for Infrarust WASM plugins (`infrarust:plugin@0.1.0`).
//!
//! This crate carries no runtime code; it owns the `wit/` directory so the host
//! loader and the guest SDK generate bindings from one source of truth.

/// Semver version of the WIT world this crate ships.
pub const WORLD_VERSION: &str = "0.1.0";

/// Path to the `wit/` directory, relative to this crate's manifest dir.
pub const WIT_DIR: &str = "wit";
