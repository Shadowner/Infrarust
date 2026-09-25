use infrarust_api::events::proxy::{
    BackendHealthEvent, ConfigReloadEvent, PingResponse, ProxyInitializeEvent, ProxyPingEvent,
    ProxyShutdownEvent, ServerStateChangeEvent,
};
use infrarust_api::services::load_balancer::BackendState;
use infrarust_api::types::ProtocolVersion;

use super::{Applied, Texts, WasmEvent, unmatched};
use crate::bindings::infrarust::plugin::events::{self as we, EventKind};
use crate::component;
use crate::convert;

fn ping_response_to_wit(response: &PingResponse) -> we::ProxyPingResult {
    we::ProxyPingResult {
        description: component::to_wit(&response.description),
        max_players: response.max_players,
        online_players: response.online_players,
        protocol: response.protocol_version.raw(),
        version_name: response.version_name.clone(),
        favicon: response.favicon.clone(),
        player_sample: response
            .player_sample
            .iter()
            .map(|(name, uuid)| we::PingPlayer {
                name: name.clone(),
                uuid: convert::uuid_to_wit(*uuid),
            })
            .collect(),
    }
}

impl WasmEvent for ProxyPingEvent {
    const KIND: EventKind = EventKind::ProxyPing;

    fn to_wit(&self) -> we::Event {
        we::Event::ProxyPing(we::ProxyPingEvent {
            remote_addr: convert::socket_to_wit(self.remote_addr),
            server: self.server.as_ref().map(|s| s.as_str().to_owned()),
            virtual_host: self.virtual_host.clone(),
            protocol: self.protocol_version.raw(),
            legacy: self.legacy,
            result: ping_response_to_wit(&self.response),
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::ProxyPing(result) = outcome else {
            return unmatched(&outcome);
        };
        let mut texts = Texts::default();
        let response = &mut self.response;
        if result.description != component::to_wit(&response.description) {
            response.description = texts.convert(&result.description);
        }
        response.max_players = result.max_players;
        response.online_players = result.online_players;
        response.protocol_version = ProtocolVersion::new(result.protocol);
        response.version_name = result.version_name;
        response.favicon = result.favicon;
        response.player_sample = result
            .player_sample
            .into_iter()
            .map(|player| (player.name, convert::uuid_from_wit(player.uuid)))
            .collect();
        texts.applied()
    }
}

impl WasmEvent for ProxyInitializeEvent {
    const KIND: EventKind = EventKind::ProxyInitialize;

    fn to_wit(&self) -> we::Event {
        we::Event::ProxyInitialize
    }
}

impl WasmEvent for ProxyShutdownEvent {
    const KIND: EventKind = EventKind::ProxyShutdown;

    fn to_wit(&self) -> we::Event {
        we::Event::ProxyShutdown
    }
}

impl WasmEvent for ConfigReloadEvent {
    const KIND: EventKind = EventKind::ConfigReload;

    fn to_wit(&self) -> we::Event {
        we::Event::ConfigReload(we::ConfigReloadEvent {
            provider: self.provider.clone(),
            added: convert::server_ids(&self.added),
            removed: convert::server_ids(&self.removed),
            updated: convert::server_ids(&self.updated),
        })
    }
}

impl WasmEvent for ServerStateChangeEvent {
    const KIND: EventKind = EventKind::ServerStateChange;

    fn to_wit(&self) -> we::Event {
        we::Event::ServerStateChange(we::ServerStateChangeEvent {
            server: self.server.as_str().to_owned(),
            old_state: convert::server_state_to_wit(self.old_state),
            new_state: convert::server_state_to_wit(self.new_state),
        })
    }
}

impl WasmEvent for BackendHealthEvent {
    const KIND: EventKind = EventKind::BackendHealth;

    fn to_wit(&self) -> we::Event {
        we::Event::BackendHealth(we::BackendHealthEvent {
            address: convert::server_address_to_wit(&self.address),
            servers: convert::server_ids(&self.servers),
            state: match self.state {
                BackendState::Healthy => we::BackendState::Healthy,
                BackendState::Probing => we::BackendState::Probing,
                BackendState::Draining => we::BackendState::Draining,
                _ => we::BackendState::Unhealthy,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use infrarust_api::types::{Component, HoverEvent, ServerId};

    use super::*;

    fn ping() -> ProxyPingEvent {
        let mut response = PingResponse::new(
            Component::text("motd").hover(HoverEvent::show_item("minecraft:stone", 1)),
            20,
            1,
            ProtocolVersion::new(774),
            "Infrarust".into(),
            None,
        );
        response.player_sample = vec![("Notch".to_owned(), uuid::Uuid::from_u128(7))];
        ProxyPingEvent::new(
            "127.0.0.1:25565".parse().unwrap(),
            Some(ServerId::new("lobby")),
            Some("lobby.test".into()),
            ProtocolVersion::new(774),
            true,
            response,
        )
    }

    fn current(event: &ProxyPingEvent) -> we::ProxyPingResult {
        let we::Event::ProxyPing(record) = event.to_wit() else {
            panic!("a ping is sent as proxy-ping");
        };
        assert!(record.legacy);
        assert_eq!(record.virtual_host.as_deref(), Some("lobby.test"));
        record.result
    }

    #[test]
    fn a_ping_outcome_keeps_the_player_sample_and_an_echoed_description() {
        let mut event = ping();
        let original = event.response.clone();
        let mut outcome = current(&event);
        outcome.max_players = 99;

        assert_eq!(
            event.apply(we::EventOutcome::ProxyPing(outcome)),
            Applied::Set
        );

        assert_eq!(event.response.max_players, 99);
        assert_eq!(event.response.player_sample, original.player_sample);
        assert_eq!(
            event.response.description, original.description,
            "an unchanged description keeps what the contract cannot carry"
        );
    }

    #[test]
    fn a_new_description_replaces_the_native_one() {
        let mut event = ping();
        let mut outcome = current(&event);
        outcome.description = component::to_wit(&Component::text("hello"));
        event.apply(we::EventOutcome::ProxyPing(outcome));
        assert_eq!(event.response.description, Component::text("hello"));
    }

    #[test]
    fn a_backend_health_change_names_its_servers() {
        let event = BackendHealthEvent {
            address: infrarust_api::types::ServerAddress {
                host: "10.0.0.2".into(),
                port: 25565,
            },
            servers: vec![ServerId::new("lobby")],
            state: BackendState::Draining,
        };
        let we::Event::BackendHealth(record) = event.to_wit() else {
            panic!("a backend health change is sent as backend-health");
        };
        assert_eq!(record.servers, ["lobby"]);
        assert_eq!(record.state, we::BackendState::Draining);
        assert_eq!(record.address.port, 25565);
    }
}
