//! Core proxy logic for Infrarust.
//!
//! Provides the middleware pipeline, connection handlers (passthrough, client-only, offline),
//! configuration providers (file, docker), server routing, status handling, authentication,
//! ban management, and the event bus system.

pub mod auth;
pub mod ban;
pub mod commands;
pub mod console;
pub mod error;
pub mod event_bus;
pub mod filter;
pub mod forwarding;
pub mod handler;
pub(crate) mod limbo;
pub mod middleware;
pub mod permissions;
pub mod pipeline;
pub mod player;
pub mod plugin;
pub mod provider;
pub mod registry;
pub mod registry_data;
pub mod routing;
pub mod server;
pub mod services;
pub mod session;
pub mod status;
pub mod telemetry;
pub mod util;
