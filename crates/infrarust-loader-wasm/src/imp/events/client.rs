use infrarust_api::event::ResultedEvent;
use infrarust_api::events::client::{
    PlayerChannelRegisterEvent, PlayerClientBrandEvent, PlayerSettingsChangedEvent,
};
use infrarust_api::events::resource_pack::{PlayerResourcePackStatusEvent, ResourcePackOrigin};
use infrarust_api::events::transfer::{PreTransferEvent, PreTransferResult, TransferOrigin};
use infrarust_api::player::ResourcePackStatus;

use super::{Applied, Texts, WasmEvent, unmatched};
use crate::bindings::infrarust::plugin::events::{self as we, EventKind};
use crate::bindings::infrarust::plugin::types as wt;
use crate::component;
use crate::convert;

impl WasmEvent for PlayerClientBrandEvent {
    const KIND: EventKind = EventKind::PlayerClientBrand;

    fn to_wit(&self) -> we::Event {
        we::Event::PlayerClientBrand(we::PlayerClientBrandEvent {
            player: convert::player_ref(&*self.player),
            brand: self.brand.clone(),
        })
    }
}

impl WasmEvent for PlayerSettingsChangedEvent {
    const KIND: EventKind = EventKind::PlayerSettingsChanged;

    fn to_wit(&self) -> we::Event {
        we::Event::PlayerSettingsChanged(we::PlayerSettingsChangedEvent {
            player: convert::player_ref(&*self.player),
            settings: convert::client_settings_to_wit(&self.settings),
        })
    }
}

impl WasmEvent for PlayerChannelRegisterEvent {
    const KIND: EventKind = EventKind::PlayerChannelRegister;

    fn to_wit(&self) -> we::Event {
        we::Event::PlayerChannelRegister(we::PlayerChannelRegisterEvent {
            player: convert::player_ref(&*self.player),
            channels: self.channels.clone(),
            direction: convert::packet_direction_to_wit(self.direction),
        })
    }
}

const fn pack_status(status: ResourcePackStatus) -> we::ResourcePackStatus {
    match status {
        ResourcePackStatus::SuccessfullyLoaded => we::ResourcePackStatus::SuccessfullyLoaded,
        ResourcePackStatus::Declined => we::ResourcePackStatus::Declined,
        ResourcePackStatus::FailedDownload => we::ResourcePackStatus::FailedDownload,
        ResourcePackStatus::Accepted => we::ResourcePackStatus::Accepted,
        ResourcePackStatus::Downloaded => we::ResourcePackStatus::Downloaded,
        ResourcePackStatus::InvalidUrl => we::ResourcePackStatus::InvalidUrl,
        ResourcePackStatus::FailedReload => we::ResourcePackStatus::FailedReload,
        ResourcePackStatus::Discarded => we::ResourcePackStatus::Discarded,
        other => we::ResourcePackStatus::Unknown(other.id()),
    }
}

impl WasmEvent for PlayerResourcePackStatusEvent {
    const KIND: EventKind = EventKind::PlayerResourcePackStatus;

    fn to_wit(&self) -> we::Event {
        we::Event::PlayerResourcePackStatus(we::PlayerResourcePackStatusEvent {
            player: convert::player_ref(&*self.player),
            pack_id: self.pack_id.map(convert::uuid_to_wit),
            status: pack_status(self.status),
            origin: match self.origin {
                ResourcePackOrigin::Backend => we::ResourcePackOrigin::Backend,
                _ => we::ResourcePackOrigin::Proxy,
            },
        })
    }
}

impl WasmEvent for PreTransferEvent {
    const KIND: EventKind = EventKind::PreTransfer;

    fn to_wit(&self) -> we::Event {
        we::Event::PreTransfer(we::PreTransferEvent {
            player: convert::player_ref(&*self.player),
            host: self.host.clone(),
            port: self.port,
            origin: match self.origin {
                TransferOrigin::Backend => we::TransferOrigin::Backend,
                _ => we::TransferOrigin::Plugin,
            },
            result: match self.result() {
                PreTransferResult::Denied { reason } => {
                    we::PreTransferResult::Denied(component::to_wit(reason))
                }
                PreTransferResult::Redirect { host, port } => {
                    we::PreTransferResult::Redirect(wt::ServerAddress {
                        host: host.clone(),
                        port: *port,
                    })
                }
                _ => we::PreTransferResult::Allowed,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::PreTransfer(result) = outcome else {
            return unmatched(&outcome);
        };
        let mut texts = Texts::default();
        self.set_result(match result {
            we::PreTransferResult::Allowed => PreTransferResult::Allowed,
            we::PreTransferResult::Denied(reason) => PreTransferResult::Denied {
                reason: texts.convert(&reason),
            },
            we::PreTransferResult::Redirect(target) => PreTransferResult::Redirect {
                host: target.host,
                port: target.port,
            },
        });
        texts.applied()
    }
}

#[cfg(test)]
mod tests {
    use infrarust_api::events::packet::PacketDirection;
    use infrarust_api::player::ClientSettings;
    use infrarust_api::types::Component;

    use super::super::steve;
    use super::*;

    #[test]
    fn client_state_events_reach_the_guest() {
        let we::Event::PlayerClientBrand(brand) =
            PlayerClientBrandEvent::new(steve(), "fabric".into()).to_wit()
        else {
            panic!("a brand is sent as player-client-brand");
        };
        assert_eq!(brand.brand, "fabric");

        let we::Event::PlayerSettingsChanged(settings) =
            PlayerSettingsChangedEvent::new(steve(), ClientSettings::new("de_de")).to_wit()
        else {
            panic!("settings are sent as player-settings-changed");
        };
        assert_eq!(settings.settings.locale, "de_de");
        assert_eq!(settings.settings.view_distance, 10);

        let we::Event::PlayerChannelRegister(channels) = PlayerChannelRegisterEvent::new(
            steve(),
            vec!["mod:a".into()],
            PacketDirection::Clientbound,
        )
        .to_wit() else {
            panic!("channels are sent as player-channel-register");
        };
        assert_eq!(channels.channels, ["mod:a"]);
        assert_eq!(channels.direction, wt::PacketDirection::Clientbound);
    }

    #[test]
    fn an_unknown_pack_status_keeps_its_id() {
        let event = PlayerResourcePackStatusEvent::new(
            steve(),
            Some(uuid::Uuid::from_u128(5)),
            ResourcePackStatus::from_id(42),
            ResourcePackOrigin::Backend,
        );
        let we::Event::PlayerResourcePackStatus(record) = event.to_wit() else {
            panic!("a pack status is sent as player-resource-pack-status");
        };
        assert_eq!(record.status, we::ResourcePackStatus::Unknown(42));
        assert_eq!(record.origin, we::ResourcePackOrigin::Backend);
        assert_eq!(
            record.pack_id,
            Some(convert::uuid_to_wit(uuid::Uuid::from_u128(5)))
        );
    }

    #[test]
    fn a_transfer_can_be_redirected_or_denied() {
        let mut event = PreTransferEvent::new(
            steve(),
            "old.example.com".into(),
            25565,
            TransferOrigin::Plugin,
        );
        event.apply(we::EventOutcome::PreTransfer(
            we::PreTransferResult::Redirect(wt::ServerAddress {
                host: "new.example.com".into(),
                port: 25566,
            }),
        ));
        assert_eq!(event.destination(), Some(("new.example.com", 25566)));
        let reason = component::to_wit(&Component::text("stay"));
        event.apply(we::EventOutcome::PreTransfer(
            we::PreTransferResult::Denied(reason.clone()),
        ));
        assert_eq!(event.destination(), None);
        let we::Event::PreTransfer(record) = event.to_wit() else {
            panic!("a transfer is sent as pre-transfer");
        };
        assert_eq!(record.result, we::PreTransferResult::Denied(reason));
        assert_eq!(record.host, "old.example.com");
    }
}
