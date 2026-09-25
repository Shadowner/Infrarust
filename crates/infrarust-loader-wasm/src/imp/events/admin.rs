use infrarust_api::events::ban::{BanIssuedEvent, BanRevokedEvent};
use infrarust_api::events::plugin::{PluginDisabledEvent, PluginEnabledEvent};

use super::WasmEvent;
use crate::bindings::infrarust::plugin::events::{self as we, EventKind};
use crate::convert;

impl WasmEvent for BanIssuedEvent {
    const KIND: EventKind = EventKind::BanIssued;

    fn to_wit(&self) -> we::Event {
        we::Event::BanIssued(we::BanIssuedEvent {
            entry: convert::ban_entry_to_wit(&self.entry),
            source: convert::ban_source_to_wit(&self.source),
            silent: self.silent,
        })
    }
}

impl WasmEvent for BanRevokedEvent {
    const KIND: EventKind = EventKind::BanRevoked;

    fn to_wit(&self) -> we::Event {
        we::Event::BanRevoked(we::BanRevokedEvent {
            entry: convert::ban_entry_to_wit(&self.entry),
            source: convert::ban_source_to_wit(&self.source),
            silent: self.silent,
        })
    }
}

impl WasmEvent for PluginEnabledEvent {
    const KIND: EventKind = EventKind::PluginEnabled;

    fn to_wit(&self) -> we::Event {
        we::Event::PluginEnabled(we::PluginEnabledEvent {
            plugin_id: self.plugin_id.clone(),
            version: self.version.clone(),
        })
    }
}

impl WasmEvent for PluginDisabledEvent {
    const KIND: EventKind = EventKind::PluginDisabled;

    fn to_wit(&self) -> we::Event {
        we::Event::PluginDisabled(we::PluginDisabledEvent {
            plugin_id: self.plugin_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use infrarust_api::services::ban_service::{BanEntry, BanSource, BanTarget};

    use super::*;
    use crate::bindings::infrarust::plugin::ban_service as wb;

    #[test]
    fn a_ban_event_carries_the_entry_and_who_issued_it() {
        let uuid = uuid::Uuid::from_u128(9);
        let entry = BanEntry::new(
            "b1",
            BanTarget::Username("Steve".into()),
            BanSource::Console,
        )
        .reason("griefing");
        let issued = BanIssuedEvent::new(
            entry,
            BanSource::Player {
                uuid,
                name: "Admin".into(),
            },
            true,
        );
        let we::Event::BanIssued(record) = issued.to_wit() else {
            panic!("a ban is sent as ban-issued");
        };
        assert_eq!(record.entry.id, "b1");
        assert_eq!(record.entry.reason.as_deref(), Some("griefing"));
        assert!(record.silent);
        assert_eq!(
            record.source,
            wb::BanSource::Player(wb::BanActor {
                uuid: convert::uuid_to_wit(uuid),
                name: "Admin".into()
            })
        );

        let revoked = BanRevokedEvent::new(
            BanEntry::new("b1", BanTarget::Username("Steve".into()), BanSource::System),
            BanSource::WebApi {
                actor: Some("ops".into()),
            },
            false,
        );
        let we::Event::BanRevoked(record) = revoked.to_wit() else {
            panic!("an unban is sent as ban-revoked");
        };
        assert_eq!(record.source, wb::BanSource::WebApi(Some("ops".into())));
    }

    #[test]
    fn plugin_lifecycle_events_name_the_plugin() {
        let we::Event::PluginEnabled(enabled) = PluginEnabledEvent::new("stats", "1.2.0").to_wit()
        else {
            panic!("an enable is sent as plugin-enabled");
        };
        assert_eq!(
            (enabled.plugin_id.as_str(), enabled.version.as_str()),
            ("stats", "1.2.0")
        );
        let we::Event::PluginDisabled(disabled) = PluginDisabledEvent::new("stats").to_wit() else {
            panic!("a disable is sent as plugin-disabled");
        };
        assert_eq!(disabled.plugin_id, "stats");
    }
}
