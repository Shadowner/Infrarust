//! Convenience re-exports for plugin development.
//!
//! ```ignore
//! use infrarust_api::prelude::*;
//! ```

// Core types
pub use crate::types::{
    ClickEvent, Component, GameProfile, HoverEvent, PlayerId, ProfileProperty, ProtocolVersion,
    RawPacket, ServerAddress, ServerId, TitleData,
};

// Error types
pub use crate::error::{PlayerError, PluginError, ServiceError};

// Event system
pub use crate::event::bus::{EventBus, EventBusExt};
pub use crate::event::{
    BoxFuture, ConnectionState, Event, EventPriority, ListenerHandle, PacketDirection,
    PacketFilter, ResultedEvent,
};

// Concrete events
pub use crate::events::*;

// Plugin lifecycle
pub use crate::plugin::{Plugin, PluginContext, PluginDependency, PluginMetadata};

// Player
pub use crate::player::Player;

// Services
pub use crate::services::{
    BackendState, BackendStatus, BanEntry, BanService, BanTarget, ConfigService, ConfigWriteError,
    LbError, LoadBalancerService, PlayerRegistry, PluginRegistry, ProxyInfo, ProxyMode, Scheduler,
    ServerConfig, ServerManager, ServerState, TaskHandle,
};

// Permissions and capabilities
pub use crate::permissions::{Capability, CapabilitySet, PermissionChecker, PermissionLevel};

// Limbo
pub use crate::limbo::{
    HandlerResult, LimboEntryContext, LimboHandler, LimboSession, SessionEndReason, SessionHandle,
};

// Virtual backend
pub use crate::virtual_backend::{VirtualBackendHandler, VirtualBackendSession};

// Commands
pub use crate::command::{CommandContext, CommandHandler, CommandManager};

// Proxy messages
pub use crate::message::ProxyMessage;

// Filters
pub use crate::filter::{
    CodecFilterError, CodecFilterFactory, CodecFilterInstance, CodecFilterRegistry,
    CodecSessionInit, CodecVerdict, ConnectionSide, FilterMetadata, FilterPriority, FilterVerdict,
    FrameOutput, TransportContext, TransportFilter, TransportFilterRegistry,
};

pub use crate::provider::{
    PluginConfigProvider, PluginProviderEvent, PluginProviderSender, ServerDocument,
};

// Standard library
pub use std::sync::Arc;
