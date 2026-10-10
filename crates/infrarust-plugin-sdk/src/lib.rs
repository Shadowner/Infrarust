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
//!         ctx.on::<PostLoginEvent>(EventPriority::NORMAL, |e| {
//!             info!("{} joined", e.player.username);
//!         })?;
//!         Ok(())
//!     }
//! }
//! ```

#[doc(hidden)]
pub mod __private {
    //! Component glue used by the `#[plugin]` expansion. Not part of the SDK API.

    pub mod bindings;
    pub use bindings::export;
}
pub mod ban_provider;
pub mod codec;
pub mod command;
pub mod component;
pub mod context;
pub mod error;
pub mod event;
mod host;
pub mod limbo;
pub mod log;
pub mod permissions;
pub mod player;
pub mod plugin;
mod registry;
#[doc(hidden)]
pub mod runtime;
pub mod services;
pub mod types;

use __private::bindings;

macro_rules! reexports {
    () => {
        pub use crate::ban_provider::{
            BanFeatures, BanProvider, BanQuery, BanRecord, BanRecordPage, BanVerdict, LoginAttempt,
            LoginStage, UnbanRequest,
        };
        pub use crate::codec::{
            CodecContext, CodecFilter, CodecRegistrar, CodecSessionInit, ConnectionSide,
            ConnectionState, FilterPriority, Injections, Packet, Verdict,
        };
        pub use crate::command::{
            CommandBuilder, CommandInfo, CommandInvocation, CommandRegistration, CommandSender,
            Commands, Completion, Suggestion,
        };
        pub use crate::component::{
            ClickEvent, Component, Content, Decoration, HoverEvent, IntoTextColor, NamedColor,
            Style, TextColor,
        };
        pub use crate::context::{
            Context, DisableReason, EnableReason, EventSubscription, RecoveryInfo, TaskHandle,
        };
        pub use crate::error::{Error, ErrorKind, PluginError};
        pub use crate::event::{
            EventPriority, GuestEvent, NamedEvent, NamedOutcome, NamedResponse, PacketFilter,
            ResultCell,
        };
        pub use crate::limbo::{
            EntryContext, HandlerOutcome, LimboHandler, LimboRegistrar, LimboSession,
            SessionEndReason, SessionHandle, TimeoutOutcome,
        };
        pub use crate::permissions::{
            PermissionDefault, PermissionNode, PermissionNodeInfo, PermissionProvider,
            PermissionSnapshot, PermissionSubject, Permissions, PlayerSubject,
        };
        pub use crate::player::{
            BossBar, BossBarColor, BossBarFlags, BossBarHandle, BossBarOverlay, ConnectionResult,
            Player, PlayerInfo, PlayerSummary, Players, ResourcePackRequest, TitleData,
        };
        pub use crate::plugin::{Plugin, PluginDependency, PluginMetadata};
        pub use crate::services::{
            BackendStatus, BanEntry, BanPage, BanRequest, BanTarget, Bans, Config, KeepaliveInfo,
            LoadBalancer, Messaging, PluginHealth, PluginInfo, Plugins, Proxy, ProxyDetails,
            RateLimitInfo, ServerConfig, ServerSource, ServerStatus, Servers, StatusCacheInfo,
            UnknownDomainBehavior,
        };
        pub use crate::types::{
            Capability, ChannelId, ChatMode, ClientSettings, GameProfile, MainHand,
            PacketDirection, ParticleStatus, PlayerId, PlayerRef, ProfileProperty, ProxyMode,
            ServerAddress, ServerId, ServerState, SkinParts,
        };
        pub use infrarust_plugin_macros::plugin;
        pub use uuid::Uuid;
    };
}

reexports!();
pub use infrarust_plugin_wit::WORLD_VERSION;

#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Trace) {
            $crate::log::emit($crate::log::Level::Trace, &::std::format!($($arg)*))
        }
    };
}
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Debug) {
            $crate::log::emit($crate::log::Level::Debug, &::std::format!($($arg)*))
        }
    };
}
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Info) {
            $crate::log::emit($crate::log::Level::Info, &::std::format!($($arg)*))
        }
    };
}
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Warn) {
            $crate::log::emit($crate::log::Level::Warn, &::std::format!($($arg)*))
        }
    };
}
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Error) {
            $crate::log::emit($crate::log::Level::Error, &::std::format!($($arg)*))
        }
    };
}

pub mod prelude {
    reexports!();
    pub use crate::event::*;
    pub use crate::{debug, error, info, trace, warn};
}
