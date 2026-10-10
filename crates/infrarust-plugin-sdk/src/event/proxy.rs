use std::cell::OnceCell;
use std::net::SocketAddr;

use uuid::Uuid;

use crate::bindings::events::{self as we, Event, EventKind, EventOutcome};
use crate::component::{Component, from_host};
use crate::event::GuestEvent;
use crate::types::{
    FromWit, ServerAddress, ServerId, ServerState, server_ids, socket_from_wit, uuid_from_wit,
    uuid_to_wit,
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

#[derive(Debug, Clone)]
struct HostField<T> {
    read: OnceCell<T>,
    set: Option<T>,
}

impl<T: PartialEq> HostField<T> {
    const fn new() -> Self {
        Self {
            read: OnceCell::new(),
            set: None,
        }
    }

    fn get(&self, fetch: impl FnOnce() -> T) -> &T {
        match &self.set {
            Some(value) => value,
            None => self.read.get_or_init(fetch),
        }
    }

    fn set(&mut self, value: T) {
        self.set = Some(value);
    }

    fn set_if_changed(&mut self, value: T, fetch: impl FnOnce() -> T) {
        if *self.read.get_or_init(fetch) == value {
            self.set = None;
        } else {
            self.set = Some(value);
        }
    }
}

fn fetch_description() -> Component {
    crate::host::ping_description()
        .map(from_host)
        .unwrap_or_default()
}

fn fetch_favicon() -> Option<String> {
    crate::host::ping_favicon()
}

fn fetch_player_sample() -> Vec<(String, Uuid)> {
    crate::host::ping_player_sample()
        .into_iter()
        .map(|player| (player.name, uuid_from_wit(player.uuid)))
        .collect()
}

#[non_exhaustive]
pub struct ProxyPingEvent {
    pub remote_addr: SocketAddr,
    pub server: Option<ServerId>,
    pub virtual_host: Option<String>,
    pub protocol: i32,
    pub legacy: bool,
    max_players: i32,
    online_players: i32,
    version_protocol: i32,
    version_name: String,
    description: HostField<Component>,
    favicon: HostField<Option<String>>,
    player_sample: HostField<Vec<(String, Uuid)>>,
    changed: bool,
}

impl ProxyPingEvent {
    #[must_use]
    pub const fn max_players(&self) -> i32 {
        self.max_players
    }

    pub const fn set_max_players(&mut self, max_players: i32) {
        self.max_players = max_players;
        self.changed = true;
    }

    #[must_use]
    pub const fn online_players(&self) -> i32 {
        self.online_players
    }

    pub const fn set_online_players(&mut self, online_players: i32) {
        self.online_players = online_players;
        self.changed = true;
    }

    #[must_use]
    pub const fn version_protocol(&self) -> i32 {
        self.version_protocol
    }

    pub const fn set_version_protocol(&mut self, protocol: i32) {
        self.version_protocol = protocol;
        self.changed = true;
    }

    #[must_use]
    pub fn version_name(&self) -> &str {
        &self.version_name
    }

    pub fn set_version_name(&mut self, name: impl Into<String>) {
        self.version_name = name.into();
        self.changed = true;
    }

    #[must_use]
    pub fn description(&self) -> &Component {
        self.description.get(fetch_description)
    }

    pub fn set_description(&mut self, description: impl Into<Component>) {
        self.description.set(description.into());
        self.changed = true;
    }

    #[must_use]
    pub fn favicon(&self) -> Option<&str> {
        self.favicon.get(fetch_favicon).as_deref()
    }

    pub fn set_favicon(&mut self, favicon: Option<String>) {
        self.favicon.set(favicon);
        self.changed = true;
    }

    #[must_use]
    pub fn player_sample(&self) -> &[(String, Uuid)] {
        self.player_sample.get(fetch_player_sample)
    }

    pub fn set_player_sample(&mut self, sample: Vec<(String, Uuid)>) {
        self.player_sample.set(sample);
        self.changed = true;
    }

    #[must_use]
    pub fn response(&self) -> PingResponse {
        PingResponse {
            description: self.description().clone(),
            max_players: self.max_players,
            online_players: self.online_players,
            protocol: self.version_protocol,
            version_name: self.version_name.clone(),
            favicon: self.favicon().map(str::to_owned),
            player_sample: self.player_sample().to_vec(),
        }
    }

    pub fn set_response(&mut self, response: PingResponse) {
        self.max_players = response.max_players;
        self.online_players = response.online_players;
        self.version_protocol = response.protocol;
        self.version_name = response.version_name;
        self.description
            .set_if_changed(response.description, fetch_description);
        self.favicon.set_if_changed(response.favicon, fetch_favicon);
        self.player_sample
            .set_if_changed(response.player_sample, fetch_player_sample);
        self.changed = true;
    }

    fn read_everything(&self) {
        let _ = (self.description(), self.favicon(), self.player_sample());
    }
}

impl std::fmt::Debug for ProxyPingEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyPingEvent")
            .field("remote_addr", &self.remote_addr)
            .field("server", &self.server)
            .field("virtual_host", &self.virtual_host)
            .field("protocol", &self.protocol)
            .field("legacy", &self.legacy)
            .field("max_players", &self.max_players)
            .field("online_players", &self.online_players)
            .field("version_protocol", &self.version_protocol)
            .field("version_name", &self.version_name)
            .field("description", self.description())
            .field("favicon", &self.favicon())
            .field("player_sample", &self.player_sample())
            .finish()
    }
}

impl Clone for ProxyPingEvent {
    fn clone(&self) -> Self {
        self.read_everything();
        Self {
            remote_addr: self.remote_addr,
            server: self.server.clone(),
            virtual_host: self.virtual_host.clone(),
            protocol: self.protocol,
            legacy: self.legacy,
            max_players: self.max_players,
            online_players: self.online_players,
            version_protocol: self.version_protocol,
            version_name: self.version_name.clone(),
            description: self.description.clone(),
            favicon: self.favicon.clone(),
            player_sample: self.player_sample.clone(),
            changed: self.changed,
        }
    }
}

impl GuestEvent for ProxyPingEvent {
    const KIND: EventKind = EventKind::ProxyPing;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::ProxyPing(e) = ev else {
            return None;
        };
        Some(Self {
            remote_addr: socket_from_wit(e.remote_addr),
            server: e.server.map(ServerId::from),
            virtual_host: e.virtual_host,
            protocol: e.protocol,
            legacy: e.legacy,
            max_players: e.result.max_players,
            online_players: e.result.online_players,
            version_protocol: e.result.protocol,
            version_name: e.result.version_name,
            description: HostField::new(),
            favicon: HostField::new(),
            player_sample: HostField::new(),
            changed: false,
        })
    }

    fn into_outcome(self) -> EventOutcome {
        if !self.changed {
            return EventOutcome::Unchanged;
        }
        EventOutcome::ProxyPing(we::ProxyPingResult {
            max_players: self.max_players,
            online_players: self.online_players,
            protocol: self.version_protocol,
            version_name: self.version_name,
            description: self
                .description
                .set
                .map(|description| description.to_arena()),
            favicon: self.favicon.set,
            player_sample: self.player_sample.set.map(|sample| {
                sample
                    .into_iter()
                    .map(|(name, uuid)| we::PingPlayer {
                        name,
                        uuid: uuid_to_wit(uuid),
                    })
                    .collect()
            }),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyInitializeEvent;

guest_event!(ProxyInitializeEvent, ProxyInitialize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyShutdownEvent;

guest_event!(ProxyShutdownEvent, ProxyShutdown);

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

guest_event!(ConfigReloadEvent, ConfigReload, |e| Self {
    provider: e.provider,
    added: server_ids(e.added),
    removed: server_ids(e.removed),
    updated: server_ids(e.updated),
});

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServerStateChangeEvent {
    pub server: ServerId,
    pub old_state: ServerState,
    pub new_state: ServerState,
}

guest_event!(ServerStateChangeEvent, ServerStateChange, |e| Self {
    server: ServerId::from(e.server),
    old_state: ServerState::from_wit(e.old_state),
    new_state: ServerState::from_wit(e.new_state),
});

pub use infrarust_plugin_common::enums::BackendState;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BackendHealthEvent {
    pub address: ServerAddress,
    pub servers: Vec<ServerId>,
    pub state: BackendState,
}

guest_event!(BackendHealthEvent, BackendHealth, |e| Self {
    address: ServerAddress::from_wit(e.address),
    servers: server_ids(e.servers),
    state: BackendState::from_wit(e.state),
});

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::bindings::types as wt;
    use crate::host::with_fake;

    fn host_ping() {
        with_fake(|host| {
            host.ping_description = Some(Component::text("motd").to_arena());
            host.ping_favicon = Some("data:image/png;base64,AAAA".into());
            host.ping_sample = vec![we::PingPlayer {
                name: "Notch".into(),
                uuid: wt::Uuid { hi: 0, lo: 7 },
            }];
            host.ping_reads = 0;
        });
    }

    fn reads() -> u32 {
        with_fake(|host| host.ping_reads)
    }

    fn ping() -> ProxyPingEvent {
        host_ping();
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
                max_players: 20,
                online_players: 1,
                protocol: 767,
                version_name: "Infrarust".into(),
                description: None,
                favicon: None,
                player_sample: None,
            },
        }))
        .unwrap()
    }

    #[test]
    fn reading_the_cheap_fields_asks_the_host_nothing() {
        let event = ping();
        assert_eq!(event.max_players(), 20);
        assert_eq!(event.online_players(), 1);
        assert_eq!(event.version_protocol(), 767);
        assert_eq!(event.version_name(), "Infrarust");
        assert_eq!(reads(), 0);
        assert_eq!(event.into_outcome(), EventOutcome::Unchanged);
    }

    #[test]
    fn a_heavy_field_is_read_from_the_host_once_and_only_when_asked() {
        let event = ping();
        assert_eq!(event.description(), &Component::text("motd"));
        assert_eq!(event.description(), &Component::text("motd"));
        assert_eq!(reads(), 1);
        assert_eq!(event.favicon(), Some("data:image/png;base64,AAAA"));
        assert_eq!(
            event.player_sample(),
            [("Notch".to_owned(), Uuid::from_u128(7))]
        );
        assert_eq!(reads(), 3);
        assert_eq!(event.into_outcome(), EventOutcome::Unchanged);
    }

    #[test]
    fn changing_a_cheap_field_sends_no_heavy_field_back() {
        let mut event = ping();
        event.set_max_players(event.max_players() + 1);
        let EventOutcome::ProxyPing(result) = event.into_outcome() else {
            panic!("an edited ping answers with its changes");
        };
        assert_eq!(result.max_players, 21);
        assert_eq!(result.online_players, 1);
        assert_eq!(result.description, None);
        assert_eq!(result.favicon, None);
        assert_eq!(result.player_sample, None);
        assert_eq!(reads(), 0);
    }

    #[test]
    fn a_set_heavy_field_is_sent_without_reading_the_host_one() {
        let mut event = ping();
        event.set_description("hello");
        event.set_favicon(None);
        assert_eq!(event.description(), &Component::text("hello"));
        let EventOutcome::ProxyPing(result) = event.into_outcome() else {
            panic!("an edited ping answers with its changes");
        };
        assert_eq!(
            result.description,
            Some(Component::text("hello").to_arena())
        );
        assert_eq!(result.favicon, Some(None));
        assert_eq!(result.player_sample, None);
        assert_eq!(reads(), 0);
    }

    #[test]
    fn a_whole_response_sends_only_the_heavy_fields_that_differ() {
        let mut event = ping();
        let mut response = event.response();
        assert_eq!(response.description, Component::text("motd"));
        response.max_players = 99;
        response.player_sample.clear();
        event.set_response(response);
        let EventOutcome::ProxyPing(result) = event.into_outcome() else {
            panic!("an edited ping answers with its changes");
        };
        assert_eq!(result.max_players, 99);
        assert_eq!(
            result.description, None,
            "an unchanged description stays on the host"
        );
        assert_eq!(result.favicon, None);
        assert_eq!(result.player_sample, Some(Vec::new()));
    }

    #[test]
    fn a_clone_reads_the_heavy_fields_while_the_host_still_has_them() {
        let event = ping();
        let copy = event.clone();
        assert_eq!(reads(), 3);
        with_fake(|host| host.ping_description = None);
        assert_eq!(copy.description(), &Component::text("motd"));
        assert_eq!(reads(), 3);
    }

    #[test]
    fn debug_output_shows_the_heavy_fields() {
        let shown = format!("{:?}", ping());
        assert!(shown.contains("AAAA"), "{shown}");
        assert!(shown.contains("Notch"), "{shown}");
        assert!(shown.contains("max_players: 20"), "{shown}");
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
