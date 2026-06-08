//! Ergonomic guest SDK for Infrarust WASM plugins.
//!
//! Implement [`Plugin`] and tag the impl with [`#[plugin]`](plugin); the macro
//! generates the WIT component glue. Subscribe to events with
//! `ctx.on::<E>(priority, |ev| { .. })`, register commands and scheduled tasks,
//! and call host services — all without touching the raw `wit-bindgen` surface.
//!
//! ```ignore
//! use infrarust_plugin_sdk::prelude::*;
//!
//! #[derive(Default)]
//! struct MyPlugin;
//!
//! #[plugin]
//! impl Plugin for MyPlugin {
//!     fn on_enable(&self, ctx: &Context) -> Result<(), String> {
//!         ctx.on::<PostLoginEvent>(EventPriority::Normal, |e| {
//!             info!("{} joined", e.profile.username);
//!         });
//!         Ok(())
//!     }
//! }
//! ```

pub mod bindings;
pub mod codec;
pub mod component;
pub mod context;
pub mod event;
pub mod log;
pub mod plugin;
#[doc(hidden)]
pub mod runtime;
pub mod services;

pub use bindings::export;
pub use codec::{
    CodecContext, CodecFilter, CodecRegistrar, CodecSessionInit, ConnectionSide, ConnectionState,
    FilterPriority, Injections, Packet, Verdict,
};
pub use component::Component;
pub use context::{CommandBuilder, CommandInvocation, Context, EventSubscription, TaskHandle};
pub use event::{EventPriority, GuestEvent};
pub use infrarust_plugin_macros::plugin;
pub use plugin::{Plugin, PluginDependency, PluginMetadata};

#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => { $crate::log::trace(&::std::format!($($arg)*)) };
}
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => { $crate::log::debug(&::std::format!($($arg)*)) };
}
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { $crate::log::info(&::std::format!($($arg)*)) };
}
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { $crate::log::warn(&::std::format!($($arg)*)) };
}
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => { $crate::log::error(&::std::format!($($arg)*)) };
}

pub mod prelude {
    pub use crate::codec::{
        CodecContext, CodecFilter, CodecRegistrar, CodecSessionInit, ConnectionState,
        FilterPriority, Injections, Packet, Verdict,
    };
    pub use crate::component::Component;
    pub use crate::context::{CommandInvocation, Context, EventSubscription, TaskHandle};
    pub use crate::event::*;
    pub use crate::plugin::{Plugin, PluginMetadata};
    pub use crate::services::{Bans, Config, Player, Players, Servers};
    pub use crate::{debug, error, info, plugin, trace, warn};
}
