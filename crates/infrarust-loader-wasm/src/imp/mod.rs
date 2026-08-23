//! Everything wasmtime-backed lives under this module so the `wasm` feature is
//! gated once at the crate root; new submodules are gated by construction.
//!
//! Modules are `pub(crate)` and glob re-exported from `lib.rs` so internal
//! `crate::xxx` paths keep resolving at the crate root.

pub(crate) mod bindings;
pub(crate) mod cache;
pub(crate) mod codec;
pub(crate) mod consts;
pub(crate) mod convert;
pub(crate) mod dispatch;
pub(crate) mod engine;
pub(crate) mod epoch;
pub(crate) mod error;
pub(crate) mod hosts;
pub(crate) mod limbo;
pub(crate) mod linker;
pub(crate) mod loader;
pub(crate) mod metadata;
pub(crate) mod plugin;
pub(crate) mod proxies;
pub(crate) mod resources;
pub(crate) mod store_state;

pub use engine::build_engine;
pub use error::WasmLoaderError;
pub use loader::WasmPluginLoader;
