//! Core proxy logic for Infrarust.
//!
//! Provides the middleware pipeline, connection handlers (passthrough, client-only, offline),
//! configuration providers (file, docker), server routing, status handling, authentication,
//! ban management, and the event bus system.

pub mod auth;
pub mod ban;
pub mod error;
pub mod event_bus;
pub mod handler;
pub mod middleware;
pub mod pipeline;
pub mod provider;
pub mod registry;
pub mod routing;
pub mod server;
pub mod session;
pub mod status;
pub mod telemetry;
pub mod util;
