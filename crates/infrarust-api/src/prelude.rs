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
pub use crate::event::bus::{EventBus, EventBusExt, FireError};
pub use crate::event::{
    BoxFuture, ConnectionState, Event, EventPriority, ListenerHandle, PacketDirection,
    PacketFilter, ResultedEvent,
};

// Concrete events
pub use crate::events::*;

// Plugin lifecycle
pub use crate::plugin::{Plugin, PluginContext, PluginDependency, PluginMetadata};

// Player
pub use crate::player::{
    BossBar, BossBarColor, BossBarFlags, BossBarHandle, BossBarOverlay, BossBarUpdate, ChatMode,
    ClientSettings, ConnectionResult, MainHand, ParticleStatus, Player, ResourcePackRequest,
    ResourcePackStatus, SkinParts,
};

pub use crate::messaging::{
    ChannelId, ChannelIdError, ChannelRegistrar, Endpoint, MessagePhase, MessagingError,
    ServerMessenger,
};

// Services
pub use crate::services::{
    BackendState, BackendStatus, BanEntry, BanFeatures, BanPage, BanProvider, BanProviderRejected,
    BanQuery, BanRequest, BanService, BanSource, BanTarget, BanVerdict, ConfigService,
    ConfigWriteError, LbError, LoadBalancerService, LoginAttempt, LoginStage, PlayerRegistry,
    PluginRegistry, ProxyInfo, ProxyMode, Scheduler, ServerConfig, ServerManager, ServerState,
    ServiceHandle, ServiceRegistry, ServiceRegistryExt, TaskHandle, UnbanRequest,
};

// Permissions and capabilities
pub use crate::permissions::{
    ADMIN_PERMISSION, Capability, CapabilitySet, PermissionChecker, PermissionDefault,
    PermissionMap, PermissionNode, PermissionNodeError, PermissionNodeInfo, PermissionProvider,
    PermissionProviderRejected, PermissionSnapshot, PermissionSubject, SnapshotPermissionChecker,
    Tristate,
};

// Limbo
pub use crate::limbo::{
    HandlerResult, LimboEntryContext, LimboHandler, LimboHandlerError, LimboHandlerRegistration,
    LimboSession, SessionEndReason, SessionHandle,
};

// Virtual backend
pub use crate::virtual_backend::{VirtualBackendHandler, VirtualBackendSession};

// Commands
pub use crate::command::{
    CommandContext, CommandError, CommandHandler, CommandInfo, CommandManager, CommandRegistration,
    CommandSource, CommandSpec, SuggestContext, Suggestion,
};

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
