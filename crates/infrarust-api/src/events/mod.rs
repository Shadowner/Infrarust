//! Concrete event definitions.
//!
//! Events are grouped by category:
//! - [`lifecycle`] — Login, post-login, disconnect
//! - [`connection`] — Server routing and kicks
//! - [`proxy`] — Proxy-level events (ping, init, shutdown, config)
//! - [`chat`] — Chat message interception
//! - [`packet`] — Raw packet events (Tier 3)

use std::sync::Arc;

use crate::event::Event;
use crate::player::Player;
use crate::types::{GameProfile, PlayerId};

pub mod ban;
pub mod chat;
pub mod client;
pub mod command;
pub mod connection;
pub mod handshake;
pub mod lifecycle;
pub mod limbo;
pub mod messaging;
pub mod named;
pub mod packet;
pub mod plugin;
pub mod proxy;
pub mod resource_pack;
pub mod transfer;

pub use ban::{BanIssuedEvent, BanRevokedEvent};
pub use chat::{ChatMessageEvent, ChatMessageResult};
pub use client::{PlayerChannelRegisterEvent, PlayerClientBrandEvent, PlayerSettingsChangedEvent};
pub use command::{CommandExecuteEvent, CommandExecuteResult};
pub use connection::{
    ConnectCause, KickCause, KickedFromServerEvent, KickedFromServerResult,
    PlayerChooseInitialServerEvent, PlayerChooseInitialServerResult, ServerConnectedEvent,
    ServerPostConnectEvent, ServerPreConnectEvent, ServerPreConnectResult,
};
pub use handshake::{
    ConnectionHandshakeEvent, ConnectionHandshakeResult, ConnectionRejectedEvent, HandshakeIntent,
    RejectReason,
};
pub use lifecycle::{
    DisconnectCause, DisconnectEvent, GameProfileRequestEvent, LoginEvent, LoginResult,
    OnlineAuthFailedEvent, PermissionsSetupEvent, PermissionsSetupResult, PostLoginEvent,
    PreLoginEvent, PreLoginResult,
};
pub use limbo::{LimboEnterEvent, LimboExitEvent, LimboExitReason};
pub use messaging::{PluginMessageEvent, PluginMessageResult};
pub use named::{NamedEvent, NamedEventResponse};
pub use packet::{PacketDirection, RawPacketEvent, RawPacketResult};
pub use plugin::{
    PluginDisabledEvent, PluginEnabledEvent, ServiceProvidedEvent, ServiceRemovedEvent,
};
pub use proxy::{
    BackendHealthEvent, ConfigReloadEvent, PingResponse, ProxyInitializeEvent, ProxyPingEvent,
    ProxyShutdownEvent, ServerStateChangeEvent,
};
pub use resource_pack::{PlayerResourcePackStatusEvent, ResourcePackOrigin};
pub use transfer::{PreTransferEvent, PreTransferResult, TransferOrigin};

pub trait PlayerEvent: Event {
    fn player(&self) -> &Arc<dyn Player>;

    fn player_id(&self) -> PlayerId {
        self.player().id()
    }

    fn profile(&self) -> &GameProfile {
        self.player().profile()
    }
}

macro_rules! player_event {
    ($event:ty) => {
        impl $crate::events::PlayerEvent for $event {
            fn player(&self) -> &::std::sync::Arc<dyn $crate::player::Player> {
                &self.player
            }
        }

        impl $event {
            pub fn player_id(&self) -> $crate::types::PlayerId {
                self.player.id()
            }
        }
    };
    ($event:ty, profile) => {
        $crate::events::player_event!($event);

        impl $event {
            pub fn profile(&self) -> &$crate::types::GameProfile {
                self.player.profile()
            }
        }
    };
}

pub(crate) use player_event;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::test_util::MockPlayer;
    use crate::types::ServerId;

    fn describe<E: PlayerEvent>(event: &E) -> (PlayerId, String) {
        (event.player_id(), event.profile().username.clone())
    }

    #[test]
    fn player_events_answer_through_the_trait_and_inherently() {
        let steve: Arc<dyn Player> = MockPlayer::new(7, "Steve").into_arc();
        let event = PostLoginEvent::new(Arc::clone(&steve));
        assert_eq!(describe(&event), (PlayerId::new(7), "Steve".to_string()));
        assert_eq!(event.player_id(), PlayerId::new(7));
        assert_eq!(event.player().id(), PlayerId::new(7));

        let switch = ServerPostConnectEvent::new(steve, ServerId::new("lobby"), None);
        assert_eq!(describe(&switch).0, PlayerId::new(7));
    }
}
