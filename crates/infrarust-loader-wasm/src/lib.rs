//! WebAssembly Component Model plugin loader for Infrarust.
//!
//! Bridges the native plugin API to a WIT boundary so `wasm32-wasip2`
//! components load through the existing `PluginLoader` trait. All wasmtime code
//! is behind the `wasm` feature; without it this crate is empty.

#[cfg(feature = "wasm")]
mod bindings;
#[cfg(feature = "wasm")]
mod cache;
#[cfg(feature = "wasm")]
mod consts;
#[cfg(feature = "wasm")]
mod engine;
#[cfg(feature = "wasm")]
mod epoch;
#[cfg(feature = "wasm")]
mod error;
#[cfg(feature = "wasm")]
mod linker;
#[cfg(feature = "wasm")]
mod loader;
#[cfg(feature = "wasm")]
mod metadata;
#[cfg(feature = "wasm")]
mod plugin;
#[cfg(feature = "wasm")]
mod store_state;

#[cfg(feature = "wasm")]
pub use engine::build_engine;
#[cfg(feature = "wasm")]
pub use error::WasmLoaderError;
#[cfg(feature = "wasm")]
pub use loader::WasmPluginLoader;
