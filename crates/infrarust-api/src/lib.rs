//! # infrarust-api
//!
//! The public plugin API for [Infrarust](https://github.com/Shadowner/Infrarust),
//! a Minecraft reverse proxy written in Rust.
//!
//! This crate defines the stable surface that plugin developers import.
//! It contains **traits, types, enums, and documentation only** — no
//! concrete proxy implementation.
//!
//! ## Quick Start
//!
//! ```no_run
//! use infrarust_api::prelude::*;
//!
//! struct MyPlugin;
//!
//! impl Plugin for MyPlugin {
//!     fn metadata(&self) -> PluginMetadata {
//!         PluginMetadata::new("my_plugin", "My Plugin", "1.0.0")
//!             .author("You")
//!             .description("An example plugin")
//!     }
//!
//!     fn on_enable<'a>(
//!         &'a self,
//!         ctx: &'a dyn PluginContext,
//!     ) -> BoxFuture<'a, Result<(), PluginError>> {
//!         Box::pin(async move {
//!             ctx.event_bus().subscribe::<PostLoginEvent, _>(
//!                 EventPriority::NORMAL,
//!                 |event| {
//!                     tracing::info!("Player joined: {}", event.profile.username);
//!                 },
//!             );
//!             Ok(())
//!         })
//!     }
//! }
//! ```
//!
//! ## Plugin Tiers
//!
//! | Tier | Capability | Key Traits |
//! |------|-----------|------------|
//! | 1 | Event listeners, commands | [`Plugin`](plugin::Plugin), [`EventBus`](event::bus::EventBus) |
//! | 2 | Limbo handlers (proxy handles protocol) | [`LimboHandler`](limbo::LimboHandler) |
//! | 3 | Virtual backends (full packet control) — *planned, not yet dispatched by the proxy* | [`VirtualBackendHandler`](virtual_backend::VirtualBackendHandler) |
//!
//! ## Modules
//!
//! - [`types`] — Domain types (identifiers, components, packets)
//! - [`event`] — Event system infrastructure
//! - [`events`] — Concrete event definitions
//! - [`filter`] — Codec and transport filter system
//! - [`plugin`] — Plugin trait and lifecycle
//! - [`player`] — Player trait
//! - [`permissions`] — Permission levels and plugin capabilities
//! - [`services`] — Proxy service traits
//! - [`limbo`] — Limbo handler system (Tier 2)
//! - [`virtual_backend`] — Virtual backend system (Tier 3, planned)
//! - [`command`] — Command system
//! - [`loader`] — Plugin discovery and loading traits
//! - [`message`] — Proxy-branded chat message helpers
//! - [`provider`] — Plugin-provided server config sources
//! - [`error`] — Error types
//! - [`prelude`] — Convenience re-exports

pub mod command;
pub mod error;
pub mod event;
pub mod events;
pub mod filter;
pub mod limbo;
pub mod loader;
pub mod message;
pub mod permissions;
pub mod player;
pub mod plugin;
pub mod prelude;
pub mod provider;
pub mod services;
pub mod types;
pub mod virtual_backend;
