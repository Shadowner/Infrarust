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
//! ```ignore
//! use infrarust_api::prelude::*;
//!
//! struct MyPlugin;
//!
//! impl Plugin for MyPlugin {
//!     fn metadata(&self) -> PluginMetadata {
//!         PluginMetadata {
//!             id: "my_plugin".into(),
//!             name: "My Plugin".into(),
//!             version: "1.0.0".into(),
//!             authors: vec!["You".into()],
//!             description: Some("An example plugin".into()),
//!             dependencies: vec![],
//!         }
//!     }
//!
//!     async fn on_enable(&self, ctx: &dyn PluginContext) -> Result<(), PluginError> {
//!         ctx.event_bus().subscribe::<PostLoginEvent>(
//!             EventPriority::NORMAL,
//!             |event| {
//!                 tracing::info!("Player joined: {}", event.profile.username);
//!             },
//!         );
//!         Ok(())
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
//! | 3 | Virtual backends (full packet control) | [`VirtualBackendHandler`](virtual_backend::VirtualBackendHandler) |
//!
//! ## Modules
//!
//! - [`types`] — Domain types (identifiers, components, packets)
//! - [`event`] — Event system infrastructure
//! - [`events`] — Concrete event definitions
//! - [`filter`] — Codec and transport filter system
//! - [`plugin`] — Plugin trait and lifecycle
//! - [`player`] — Player trait
//! - [`services`] — Proxy service traits
//! - [`limbo`] — Limbo handler system (Tier 2)
//! - [`virtual_backend`] — Virtual backend system (Tier 3)
//! - [`command`] — Command system
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
