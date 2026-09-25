//! Ergonomic guest SDK for Infrarust WASM plugins (`infrarust:plugin@0.3.0`).
//!
//! Implement [`Plugin`] and tag the impl with [`#[plugin]`](plugin); the macro
//! generates the WIT component glue.
//!
//! ```ignore
//! use infrarust_plugin_sdk::prelude::*;
//!
//! #[derive(Default)]
//! struct MyPlugin;
//!
//! #[plugin(id = "my-plugin")]
//! impl Plugin for MyPlugin {
//!     fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
//!         ctx.on::<PostLoginEvent>(EventPriority::Normal, |e| {
//!             info!("{} joined", e.player.username);
//!         })?;
//!         Ok(())
//!     }
//! }
//! ```

pub mod bindings;
pub mod codec;
pub mod command;
pub mod component;
pub mod context;
pub mod error;
pub mod event;
mod host;
pub mod limbo;
pub mod log;
pub mod player;
pub mod plugin;
mod registry;
#[doc(hidden)]
pub mod runtime;
pub mod services;
pub mod types;

pub use bindings::export;
pub use codec::{
    CodecContext, CodecFilter, CodecRegistrar, CodecSessionInit, ConnectionSide, ConnectionState,
    FilterPriority, Injections, Packet, Verdict,
};
pub use command::{
    CommandBuilder, CommandInvocation, CommandRegistration, CommandSender, Completion, Suggestion,
};
pub use component::{
    ClickEvent, Component, Content, Decoration, HoverEvent, IntoTextColor, NamedColor, Style,
    TextColor,
};
pub use context::{
    Context, DisableReason, EnableReason, EventSubscription, RecoveryInfo, TaskHandle,
};
pub use error::{Error, ErrorKind, PluginError};
pub use event::{
    EventPriority, GuestEvent, NamedEvent, NamedOutcome, NamedResponse, PacketFilter, ResultCell,
};
pub use infrarust_plugin_macros::plugin;
pub use infrarust_plugin_wit::WORLD_VERSION;
pub use limbo::{
    EntryContext, HandlerOutcome, LimboHandler, LimboRegistrar, LimboSession, SessionEndReason,
    SessionHandle, TimeoutOutcome,
};
pub use player::{
    BossBar, BossBarColor, BossBarFlags, BossBarHandle, BossBarOverlay, ConnectionResult, Player,
    PlayerInfo, Players, ResourcePackRequest, TitleData,
};
pub use plugin::{Plugin, PluginDependency, PluginMetadata};
pub use services::{
    BackendStatus, BanEntry, BanPage, BanRequest, BanTarget, Bans, Config, KeepaliveInfo,
    LoadBalancer, Messaging, PluginInfo, Plugins, Proxy, ProxyDetails, RateLimitInfo, ServerConfig,
    ServerSource, ServerStatus, Servers, StatusCacheInfo, UnknownDomainBehavior,
};
pub use types::{
    Capability, ChannelId, ChatMode, ClientSettings, GameProfile, MainHand, PacketDirection,
    ParticleStatus, PlayerId, PlayerRef, ProfileProperty, ProxyMode, ServerAddress, ServerId,
    ServerState, SkinParts,
};
pub use uuid::Uuid;

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
        CodecContext, CodecFilter, CodecRegistrar, CodecSessionInit, ConnectionSide,
        ConnectionState, FilterPriority, Injections, Packet, Verdict,
    };
    pub use crate::command::{
        CommandInvocation, CommandRegistration, CommandSender, Completion, Suggestion,
    };
    pub use crate::component::{
        ClickEvent, Component, Decoration, HoverEvent, NamedColor, TextColor,
    };
    pub use crate::context::{
        Context, DisableReason, EnableReason, EventSubscription, RecoveryInfo, TaskHandle,
    };
    pub use crate::error::{Error, ErrorKind, PluginError};
    pub use crate::event::*;
    pub use crate::limbo::{
        EntryContext, HandlerOutcome, LimboHandler, LimboRegistrar, LimboSession, SessionEndReason,
        SessionHandle, TimeoutOutcome,
    };
    pub use crate::player::{
        BossBar, BossBarColor, BossBarFlags, BossBarHandle, BossBarOverlay, ConnectionResult,
        Player, PlayerInfo, Players, ResourcePackRequest, TitleData,
    };
    pub use crate::plugin::{Plugin, PluginDependency, PluginMetadata};
    pub use crate::services::{
        BackendStatus, BanEntry, BanPage, BanRequest, BanTarget, Bans, Config, LoadBalancer,
        Messaging, PluginInfo, Plugins, Proxy, ProxyDetails, ServerConfig, ServerSource,
        ServerStatus, Servers,
    };
    pub use crate::types::{
        Capability, ChannelId, ChatMode, ClientSettings, GameProfile, MainHand, PacketDirection,
        ParticleStatus, PlayerId, PlayerRef, ProxyMode, ServerAddress, ServerId, ServerState,
        SkinParts,
    };
    pub use crate::{Uuid, debug, error, info, plugin, trace, warn};
}
