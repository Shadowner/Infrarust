use uuid::Uuid;

use super::ResultCell;
use crate::bindings::events as we;
use crate::component::{Component, from_host};
use crate::types::{
    ClientSettings, FromWit, PacketDirection, PlayerRef, ServerAddress, uuid_from_wit,
};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlayerClientBrandEvent {
    pub player: PlayerRef,
    pub brand: String,
}

guest_event!(PlayerClientBrandEvent, PlayerClientBrand, |e| Self {
    player: PlayerRef::from_wit(e.player),
    brand: e.brand,
});

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlayerSettingsChangedEvent {
    pub player: PlayerRef,
    pub settings: ClientSettings,
}

guest_event!(PlayerSettingsChangedEvent, PlayerSettingsChanged, |e| {
    Self {
        player: PlayerRef::from_wit(e.player),
        settings: ClientSettings::from_wit(e.settings),
    }
});

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlayerChannelRegisterEvent {
    pub player: PlayerRef,
    pub channels: Vec<String>,
    pub direction: PacketDirection,
}

guest_event!(PlayerChannelRegisterEvent, PlayerChannelRegister, |e| {
    Self {
        player: PlayerRef::from_wit(e.player),
        channels: e.channels,
        direction: PacketDirection::from_wit(e.direction),
    }
});

pub use infrarust_plugin_common::enums::ResourcePackStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ResourcePackOrigin {
    Proxy,
    Backend,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlayerResourcePackStatusEvent {
    pub player: PlayerRef,
    pub pack_id: Option<Uuid>,
    pub status: ResourcePackStatus,
    pub origin: ResourcePackOrigin,
}

guest_event!(
    PlayerResourcePackStatusEvent,
    PlayerResourcePackStatus,
    |e| Self {
        player: PlayerRef::from_wit(e.player),
        pack_id: e.pack_id.map(uuid_from_wit),
        status: ResourcePackStatus::from_wit(e.status),
        origin: match e.origin {
            we::ResourcePackOrigin::Proxy => ResourcePackOrigin::Proxy,
            we::ResourcePackOrigin::Backend => ResourcePackOrigin::Backend,
        },
    }
);

pub use infrarust_plugin_common::enums::TransferOrigin;

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PreTransferResult {
    Allowed,
    Denied(Component),
    Redirect(ServerAddress),
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PreTransferEvent {
    pub player: PlayerRef,
    pub host: String,
    pub port: u16,
    pub origin: TransferOrigin,
    result: ResultCell<PreTransferResult>,
}

impl PreTransferEvent {
    #[must_use]
    pub const fn result(&self) -> &PreTransferResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: PreTransferResult) {
        self.result.set(result);
    }

    pub fn allow(&mut self) {
        self.set_result(PreTransferResult::Allowed);
    }

    pub fn deny(&mut self, reason: impl Into<Component>) {
        self.set_result(PreTransferResult::Denied(reason.into()));
    }

    pub fn redirect(&mut self, host: impl Into<String>, port: u16) {
        self.set_result(PreTransferResult::Redirect(ServerAddress::new(host, port)));
    }
}

guest_event!(
    PreTransferEvent,
    PreTransfer,
    |e| Self {
        player: PlayerRef::from_wit(e.player),
        host: e.host,
        port: e.port,
        origin: TransferOrigin::from_wit(e.origin),
        result: ResultCell::new(match e.result {
            we::PreTransferResult::Allowed => PreTransferResult::Allowed,
            we::PreTransferResult::Denied(reason) => {
                PreTransferResult::Denied(from_host(reason))
            }
            we::PreTransferResult::Redirect(target) => {
                PreTransferResult::Redirect(ServerAddress::from_wit(target))
            }
        }),
    },
    result,
    |r| match r {
        PreTransferResult::Allowed => we::PreTransferResult::Allowed,
        PreTransferResult::Denied(reason) => {
            we::PreTransferResult::Denied(reason.to_arena())
        }
        PreTransferResult::Redirect(target) => {
            we::PreTransferResult::Redirect(target.to_wit())
        }
    }
);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::bindings::events::{Event, EventOutcome};
    use crate::bindings::types as wt;
    use crate::event::GuestEvent;

    #[test]
    fn a_transfer_redirect_becomes_the_outcome() {
        let mut event = PreTransferEvent::from_event(Event::PreTransfer(we::PreTransferEvent {
            player: wt::PlayerRef {
                id: 1,
                uuid: wt::Uuid { hi: 0, lo: 1 },
                username: "Steve".into(),
            },
            host: "old.example.com".into(),
            port: 25565,
            origin: we::TransferOrigin::Backend,
            result: we::PreTransferResult::Allowed,
        }))
        .unwrap();
        assert_eq!(event.origin, TransferOrigin::Backend);
        event.redirect("new.example.com", 25566);
        assert_eq!(
            event.into_outcome(),
            EventOutcome::PreTransfer(we::PreTransferResult::Redirect(wt::ServerAddress {
                host: "new.example.com".into(),
                port: 25566
            }))
        );
    }

    #[test]
    fn an_unknown_pack_status_is_not_final() {
        assert!(!ResourcePackStatus::from_wit(we::ResourcePackStatus::Unknown(9)).is_final());
        assert!(ResourcePackStatus::from_wit(we::ResourcePackStatus::Declined).is_final());
    }
}
