use std::net::SocketAddr;

use uuid::Uuid;

use super::{GuestEvent, ResultCell};
use crate::bindings::events::{self as we, Event, EventKind, EventOutcome};
use crate::component::{Component, from_host};
use crate::types::{
    ServerAddress, ServerId, ServerState, server_ids, socket_from_wit, uuid_from_wit, uuid_to_wit,
};

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PingResponse {
    pub description: Component,
    pub max_players: i32,
    pub online_players: i32,
    pub protocol: i32,
    pub version_name: String,
    pub favicon: Option<String>,
    pub player_sample: Vec<(String, Uuid)>,
}

impl PingResponse {
    fn from_wit(response: we::ProxyPingResult) -> Self {
        Self {
            description: from_host(response.description),
            max_players: response.max_players,
            online_players: response.online_players,
            protocol: response.protocol,
            version_name: response.version_name,
            favicon: response.favicon,
            player_sample: response
                .player_sample
                .into_iter()
                .map(|player| (player.name, uuid_from_wit(player.uuid)))
                .collect(),
        }
    }

    fn to_wit(&self) -> we::ProxyPingResult {
        we::ProxyPingResult {
            description: self.description.to_arena(),
            max_players: self.max_players,
            online_players: self.online_players,
            protocol: self.protocol,
            version_name: self.version_name.clone(),
            favicon: self.favicon.clone(),
            player_sample: self
                .player_sample
                .iter()
                .map(|(name, uuid)| we::PingPlayer {
                    name: name.clone(),
                    uuid: uuid_to_wit(*uuid),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ProxyPingEvent {
    pub remote_addr: SocketAddr,
    pub server: Option<ServerId>,
    pub virtual_host: Option<String>,
    pub protocol: i32,
    pub legacy: bool,
    response: ResultCell<PingResponse>,
}

impl ProxyPingEvent {
    #[must_use]
    pub const fn response(&self) -> &PingResponse {
        self.response.get()
    }

    pub fn response_mut(&mut self) -> &mut PingResponse {
        self.response.get_mut()
    }

    pub fn set_response(&mut self, response: PingResponse) {
        self.response.set(response);
    }
}

impl GuestEvent for ProxyPingEvent {
    const KIND: EventKind = EventKind::ProxyPing;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::ProxyPing(e) = ev else { return None };
        Some(Self {
            remote_addr: socket_from_wit(e.remote_addr),
            server: e.server.map(ServerId::from),
            virtual_host: e.virtual_host,
            protocol: e.protocol,
            legacy: e.legacy,
            response: ResultCell::new(PingResponse::from_wit(e.result)),
        })
    }

    fn into_outcome(self) -> EventOutcome {
        self.response
            .into_changed()
            .map_or(EventOutcome::Unchanged, |r| {
                EventOutcome::ProxyPing(r.to_wit())
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyInitializeEvent;

impl GuestEvent for ProxyInitializeEvent {
    const KIND: EventKind = EventKind::ProxyInitialize;

    fn from_event(ev: Event) -> Option<Self> {
        matches!(ev, Event::ProxyInitialize).then_some(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyShutdownEvent;

impl GuestEvent for ProxyShutdownEvent {
    const KIND: EventKind = EventKind::ProxyShutdown;

    fn from_event(ev: Event) -> Option<Self> {
        matches!(ev, Event::ProxyShutdown).then_some(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConfigReloadEvent {
    pub provider: String,
    pub added: Vec<ServerId>,
    pub removed: Vec<ServerId>,
    pub updated: Vec<ServerId>,
}

impl ConfigReloadEvent {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.updated.is_empty()
    }
}

impl GuestEvent for ConfigReloadEvent {
    const KIND: EventKind = EventKind::ConfigReload;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::ConfigReload(e) = ev else {
            return None;
        };
        Some(Self {
            provider: e.provider,
            added: server_ids(e.added),
            removed: server_ids(e.removed),
            updated: server_ids(e.updated),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServerStateChangeEvent {
    pub server: ServerId,
    pub old_state: ServerState,
    pub new_state: ServerState,
}

impl GuestEvent for ServerStateChangeEvent {
    const KIND: EventKind = EventKind::ServerStateChange;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::ServerStateChange(e) = ev else {
            return None;
        };
        Some(Self {
            server: ServerId::from(e.server),
            old_state: ServerState::from_wit(e.old_state),
            new_state: ServerState::from_wit(e.new_state),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BackendState {
    Healthy,
    Probing,
    Unhealthy,
    Draining,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BackendHealthEvent {
    pub address: ServerAddress,
    pub servers: Vec<ServerId>,
    pub state: BackendState,
}

impl GuestEvent for BackendHealthEvent {
    const KIND: EventKind = EventKind::BackendHealth;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::BackendHealth(e) = ev else {
            return None;
        };
        Some(Self {
            address: ServerAddress::from_wit(e.address),
            servers: server_ids(e.servers),
            state: match e.state {
                we::BackendState::Healthy => BackendState::Healthy,
                we::BackendState::Probing => BackendState::Probing,
                we::BackendState::Unhealthy => BackendState::Unhealthy,
                we::BackendState::Draining => BackendState::Draining,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::types as wt;

    fn ping() -> ProxyPingEvent {
        ProxyPingEvent::from_event(Event::ProxyPing(we::ProxyPingEvent {
            remote_addr: wt::SocketAddress {
                ip: wt::IpAddress::Ipv4((127, 0, 0, 1)),
                port: 25565,
            },
            server: Some("lobby".into()),
            virtual_host: None,
            protocol: 767,
            legacy: false,
            result: we::ProxyPingResult {
                description: Component::text("motd").to_arena(),
                max_players: 20,
                online_players: 1,
                protocol: 767,
                version_name: "Infrarust".into(),
                favicon: None,
                player_sample: vec![we::PingPlayer {
                    name: "Notch".into(),
                    uuid: wt::Uuid { hi: 0, lo: 7 },
                }],
            },
        }))
        .unwrap()
    }

    #[test]
    fn reading_the_response_keeps_it_unchanged() {
        let event = ping();
        assert_eq!(event.response().description, Component::text("motd"));
        assert_eq!(
            event.response().player_sample,
            [("Notch".to_owned(), Uuid::from_u128(7))]
        );
        assert_eq!(event.into_outcome(), EventOutcome::Unchanged);
    }

    #[test]
    fn editing_the_response_sends_all_of_it_back() {
        let mut event = ping();
        event.response_mut().max_players = 99;
        let EventOutcome::ProxyPing(result) = event.into_outcome() else {
            panic!("an edited ping answers with its response");
        };
        assert_eq!(result.max_players, 99);
        assert_eq!(result.description, Component::text("motd").to_arena());
        assert_eq!(result.player_sample.len(), 1);
    }

    #[test]
    fn unit_events_decode_only_their_own_kind() {
        assert_eq!(
            ProxyShutdownEvent::from_event(Event::ProxyShutdown),
            Some(ProxyShutdownEvent)
        );
        assert_eq!(ProxyShutdownEvent::from_event(Event::ProxyInitialize), None);
    }
}
