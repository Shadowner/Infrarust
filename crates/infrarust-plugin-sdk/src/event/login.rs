use std::net::SocketAddr;

use super::ResultCell;
use crate::bindings::events as we;
use crate::component::{Component, from_host};
use crate::permissions::PermissionSnapshot;
use crate::types::{GameProfile, PlayerRef, ServerId, socket_from_wit};

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PreLoginResult {
    Allowed,
    Denied(Component),
    ForceOffline,
    ForceOnline,
}

impl PreLoginResult {
    fn from_wit(result: we::PreLoginResult) -> Self {
        match result {
            we::PreLoginResult::Allowed => Self::Allowed,
            we::PreLoginResult::Denied(reason) => Self::Denied(from_host(reason)),
            we::PreLoginResult::ForceOffline => Self::ForceOffline,
            we::PreLoginResult::ForceOnline => Self::ForceOnline,
        }
    }

    fn to_wit(&self) -> we::PreLoginResult {
        match self {
            Self::Allowed => we::PreLoginResult::Allowed,
            Self::Denied(reason) => we::PreLoginResult::Denied(reason.to_arena()),
            Self::ForceOffline => we::PreLoginResult::ForceOffline,
            Self::ForceOnline => we::PreLoginResult::ForceOnline,
        }
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PreLoginEvent {
    pub profile: GameProfile,
    pub remote_addr: SocketAddr,
    pub protocol: i32,
    pub server_domain: String,
    result: ResultCell<PreLoginResult>,
}

impl PreLoginEvent {
    #[must_use]
    pub const fn result(&self) -> &PreLoginResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: PreLoginResult) {
        self.result.set(result);
    }

    pub fn allow(&mut self) {
        self.set_result(PreLoginResult::Allowed);
    }

    pub fn deny(&mut self, reason: impl Into<Component>) {
        self.set_result(PreLoginResult::Denied(reason.into()));
    }

    pub fn force_offline(&mut self) {
        self.set_result(PreLoginResult::ForceOffline);
    }

    pub fn force_online(&mut self) {
        self.set_result(PreLoginResult::ForceOnline);
    }
}

guest_event!(
    PreLoginEvent,
    PreLogin,
    |e| Self {
        profile: GameProfile::from_wit(e.profile),
        remote_addr: socket_from_wit(e.remote_addr),
        protocol: e.protocol,
        server_domain: e.server_domain,
        result: ResultCell::new(PreLoginResult::from_wit(e.result)),
    },
    result,
    |r| r.to_wit()
);

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PostLoginEvent {
    pub player: PlayerRef,
    pub profile: GameProfile,
    pub protocol: i32,
}

guest_event!(PostLoginEvent, PostLogin, |e| Self {
    player: PlayerRef::from_wit(e.player),
    profile: GameProfile::from_wit(e.profile),
    protocol: e.protocol,
});

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DisconnectCause {
    ClientQuit,
    Kicked(Option<Component>),
    BackendClosed(Option<Component>),
    Shutdown,
    Error,
}

impl DisconnectCause {
    #[must_use]
    pub const fn reason(&self) -> Option<&Component> {
        match self {
            Self::Kicked(reason) | Self::BackendClosed(reason) => reason.as_ref(),
            Self::ClientQuit | Self::Shutdown | Self::Error => None,
        }
    }

    fn from_wit(cause: we::DisconnectCause) -> Self {
        match cause {
            we::DisconnectCause::ClientQuit => Self::ClientQuit,
            we::DisconnectCause::Kicked(reason) => Self::Kicked(reason.map(from_host)),
            we::DisconnectCause::BackendClosed(reason) => {
                Self::BackendClosed(reason.map(from_host))
            }
            we::DisconnectCause::Shutdown => Self::Shutdown,
            we::DisconnectCause::Error => Self::Error,
        }
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DisconnectEvent {
    pub player: PlayerRef,
    pub last_server: Option<ServerId>,
    pub cause: DisconnectCause,
}

guest_event!(DisconnectEvent, Disconnect, |e| Self {
    player: PlayerRef::from_wit(e.player),
    last_server: e.last_server.map(ServerId::from),
    cause: DisconnectCause::from_wit(e.cause),
});

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct OnlineAuthFailedEvent {
    pub username: String,
}

guest_event!(OnlineAuthFailedEvent, OnlineAuthFailed, |e| Self {
    username: e.username,
});

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PermissionsSetupResult {
    UseDefault,
    Custom(PermissionSnapshot),
}

impl PermissionsSetupResult {
    fn from_wit(result: we::PermissionsSetupResult) -> Self {
        match result {
            we::PermissionsSetupResult::UseDefault => Self::UseDefault,
            we::PermissionsSetupResult::Custom(snapshot) => {
                Self::Custom(PermissionSnapshot::from_wit(snapshot))
            }
        }
    }

    fn to_wit(&self) -> we::PermissionsSetupResult {
        match self {
            Self::UseDefault => we::PermissionsSetupResult::UseDefault,
            Self::Custom(snapshot) => we::PermissionsSetupResult::Custom(snapshot.to_wit()),
        }
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PermissionsSetupEvent {
    pub player: PlayerRef,
    pub online_mode: bool,
    result: ResultCell<PermissionsSetupResult>,
}

impl PermissionsSetupEvent {
    #[must_use]
    pub const fn result(&self) -> &PermissionsSetupResult {
        self.result.get()
    }

    pub fn use_default(&mut self) {
        self.result.set(PermissionsSetupResult::UseDefault);
    }

    pub fn provide(&mut self, snapshot: PermissionSnapshot) {
        self.result.set(PermissionsSetupResult::Custom(snapshot));
    }
}

guest_event!(
    PermissionsSetupEvent,
    PermissionsSetup,
    |e| Self {
        player: PlayerRef::from_wit(e.player),
        online_mode: e.online_mode,
        result: ResultCell::new(PermissionsSetupResult::from_wit(e.result)),
    },
    result,
    |r| r.to_wit()
);

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum LoginResult {
    Allowed,
    Denied(Component),
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LoginEvent {
    pub player: PlayerRef,
    pub online_mode: bool,
    result: ResultCell<LoginResult>,
}

impl LoginEvent {
    #[must_use]
    pub const fn result(&self) -> &LoginResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: LoginResult) {
        self.result.set(result);
    }

    pub fn allow(&mut self) {
        self.set_result(LoginResult::Allowed);
    }

    pub fn deny(&mut self, reason: impl Into<Component>) {
        self.set_result(LoginResult::Denied(reason.into()));
    }
}

guest_event!(
    LoginEvent,
    Login,
    |e| Self {
        player: PlayerRef::from_wit(e.player),
        online_mode: e.online_mode,
        result: ResultCell::new(match e.result {
            we::LoginResult::Allowed => LoginResult::Allowed,
            we::LoginResult::Denied(reason) => LoginResult::Denied(from_host(reason)),
        }),
    },
    result,
    |r| match r {
        LoginResult::Allowed => we::LoginResult::Allowed,
        LoginResult::Denied(reason) => we::LoginResult::Denied(reason.to_arena()),
    }
);

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GameProfileRequestEvent {
    pub original: GameProfile,
    pub online_mode: bool,
    pub remote_addr: SocketAddr,
    pub virtual_host: Option<String>,
    pub protocol: i32,
    profile: ResultCell<GameProfile>,
    denied: ResultCell<Option<Component>>,
}

impl GameProfileRequestEvent {
    #[must_use]
    pub const fn profile(&self) -> &GameProfile {
        self.profile.get()
    }

    pub fn set_profile(&mut self, profile: GameProfile) {
        self.profile.set(profile);
    }

    pub fn profile_mut(&mut self) -> &mut GameProfile {
        self.profile.get_mut()
    }

    #[must_use]
    pub fn is_modified(&self) -> bool {
        *self.profile.get() != self.original
    }

    #[must_use]
    pub fn denied(&self) -> Option<&Component> {
        self.denied.get().as_ref()
    }

    pub fn deny(&mut self, reason: impl Into<Component>) {
        self.denied.set(Some(reason.into()));
    }

    pub fn allow(&mut self) {
        self.denied.set(None);
    }
}

impl crate::event::GuestEvent for GameProfileRequestEvent {
    const KIND: we::EventKind = we::EventKind::GameProfileRequest;

    fn from_event(ev: we::Event) -> Option<Self> {
        let we::Event::GameProfileRequest(e) = ev else {
            return None;
        };
        Some(Self {
            original: GameProfile::from_wit(e.original),
            online_mode: e.online_mode,
            remote_addr: socket_from_wit(e.remote_addr),
            virtual_host: e.virtual_host,
            protocol: e.protocol,
            profile: ResultCell::new(GameProfile::from_wit(e.result.profile)),
            denied: ResultCell::new(e.result.denied.map(from_host)),
        })
    }

    fn into_outcome(self) -> we::EventOutcome {
        if !self.profile.is_dirty() && !self.denied.is_dirty() {
            return we::EventOutcome::Unchanged;
        }
        we::EventOutcome::GameProfileRequest(we::GameProfileRequestResult {
            profile: self.profile.get().to_wit(),
            denied: self.denied.get().as_ref().map(Component::to_arena),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::bindings::events::{Event, EventOutcome};
    use crate::bindings::types as wt;
    use crate::event::GuestEvent;

    fn pre_login(result: we::PreLoginResult) -> Event {
        Event::PreLogin(we::PreLoginEvent {
            profile: wt::GameProfile {
                uuid: wt::Uuid { hi: 0, lo: 1 },
                username: "Steve".into(),
                properties: vec![],
            },
            remote_addr: wt::SocketAddress {
                ip: wt::IpAddress::Ipv4((203, 0, 113, 7)),
                port: 51_234,
            },
            protocol: 767,
            server_domain: "play.example.com".into(),
            result,
        })
    }

    #[test]
    fn an_untouched_event_answers_unchanged() {
        let event = PreLoginEvent::from_event(pre_login(we::PreLoginResult::ForceOnline)).unwrap();
        assert_eq!(event.result(), &PreLoginResult::ForceOnline);
        assert_eq!(event.remote_addr, "203.0.113.7:51234".parse().unwrap());
        assert_eq!(event.into_outcome(), EventOutcome::Unchanged);
    }

    #[test]
    fn allow_after_an_earlier_deny_is_sent() {
        let banned = Component::text("Banned").to_arena();
        let mut event =
            PreLoginEvent::from_event(pre_login(we::PreLoginResult::Denied(banned))).unwrap();
        assert_eq!(
            event.result(),
            &PreLoginResult::Denied(Component::text("Banned"))
        );
        event.allow();
        assert_eq!(
            event.into_outcome(),
            EventOutcome::PreLogin(we::PreLoginResult::Allowed)
        );
    }

    #[test]
    fn a_deny_carries_its_component() {
        let mut event = PreLoginEvent::from_event(pre_login(we::PreLoginResult::Allowed)).unwrap();
        event.deny(Component::text("no").bold());
        assert_eq!(
            event.into_outcome(),
            EventOutcome::PreLogin(we::PreLoginResult::Denied(
                Component::text("no").bold().to_arena()
            ))
        );
    }

    #[test]
    fn a_provided_snapshot_becomes_the_custom_outcome() {
        let setup = |result| {
            PermissionsSetupEvent::from_event(Event::PermissionsSetup(we::PermissionsSetupEvent {
                player: wt::PlayerRef {
                    id: 1,
                    uuid: wt::Uuid { hi: 0, lo: 1 },
                    username: "Steve".into(),
                },
                online_mode: true,
                result,
            }))
            .unwrap()
        };
        let mut event = setup(we::PermissionsSetupResult::UseDefault);
        assert_eq!(event.result(), &PermissionsSetupResult::UseDefault);
        let snapshot = PermissionSnapshot::new().grant("demo.use");
        event.provide(snapshot.clone());
        assert_eq!(
            event.into_outcome(),
            EventOutcome::PermissionsSetup(we::PermissionsSetupResult::Custom(snapshot.to_wit()))
        );

        let earlier = setup(we::PermissionsSetupResult::Custom(
            PermissionSnapshot::admin().to_wit(),
        ));
        assert_eq!(
            earlier.result(),
            &PermissionsSetupResult::Custom(PermissionSnapshot::admin())
        );
        assert_eq!(earlier.into_outcome(), EventOutcome::Unchanged);
    }

    #[test]
    fn an_event_of_another_kind_is_not_decoded() {
        assert!(PostLoginEvent::from_event(pre_login(we::PreLoginResult::Allowed)).is_none());
    }

    #[test]
    fn a_disconnect_exposes_its_reason() {
        let reason = Component::text("bye");
        let cause = DisconnectCause::from_wit(we::DisconnectCause::Kicked(Some(reason.to_arena())));
        assert_eq!(cause.reason(), Some(&reason));
        assert_eq!(
            DisconnectCause::from_wit(we::DisconnectCause::Shutdown).reason(),
            None
        );
    }

    #[test]
    fn a_login_deny_becomes_the_outcome() {
        let mut event = LoginEvent::from_event(Event::Login(we::LoginEvent {
            player: wt::PlayerRef {
                id: 1,
                uuid: wt::Uuid { hi: 0, lo: 1 },
                username: "Steve".into(),
            },
            online_mode: true,
            result: we::LoginResult::Allowed,
        }))
        .unwrap();
        assert_eq!(event.result(), &LoginResult::Allowed);
        event.deny("closed");
        assert_eq!(
            event.into_outcome(),
            EventOutcome::Login(we::LoginResult::Denied(
                Component::text("closed").to_arena()
            ))
        );
    }

    #[test]
    fn a_profile_edit_is_sent_back_and_an_untouched_one_is_unchanged() {
        let profile = wt::GameProfile {
            uuid: wt::Uuid { hi: 0, lo: 1 },
            username: "Steve".into(),
            properties: vec![],
        };
        let event = || {
            GameProfileRequestEvent::from_event(Event::GameProfileRequest(
                we::GameProfileRequestEvent {
                    original: profile.clone(),
                    online_mode: false,
                    remote_addr: wt::SocketAddress {
                        ip: wt::IpAddress::Ipv4((127, 0, 0, 1)),
                        port: 1,
                    },
                    virtual_host: None,
                    protocol: 767,
                    result: we::GameProfileRequestResult {
                        profile: profile.clone(),
                        denied: None,
                    },
                },
            ))
            .unwrap()
        };
        assert_eq!(event().into_outcome(), EventOutcome::Unchanged);
        let mut renamed = event();
        renamed.profile_mut().username = "Alex".into();
        assert!(renamed.is_modified());
        let EventOutcome::GameProfileRequest(result) = renamed.into_outcome() else {
            panic!("a profile edit answers game-profile-request");
        };
        assert_eq!(result.profile.username, "Alex");
    }

    fn profile_request(denied: Option<wt::Component>) -> GameProfileRequestEvent {
        let profile = wt::GameProfile {
            uuid: wt::Uuid { hi: 0, lo: 1 },
            username: "Steve".into(),
            properties: vec![],
        };
        GameProfileRequestEvent::from_event(Event::GameProfileRequest(
            we::GameProfileRequestEvent {
                original: profile.clone(),
                online_mode: true,
                remote_addr: wt::SocketAddress {
                    ip: wt::IpAddress::Ipv4((127, 0, 0, 1)),
                    port: 1,
                },
                virtual_host: None,
                protocol: 767,
                result: we::GameProfileRequestResult { profile, denied },
            },
        ))
        .unwrap()
    }

    #[test]
    fn a_profile_request_sees_an_earlier_deny_and_can_deny_or_allow() {
        let banned = Component::text("Banned");
        let seen = profile_request(Some(banned.to_arena()));
        assert_eq!(seen.denied(), Some(&banned));
        assert_eq!(seen.into_outcome(), EventOutcome::Unchanged);

        let mut allowed = profile_request(Some(banned.to_arena()));
        allowed.allow();
        let EventOutcome::GameProfileRequest(result) = allowed.into_outcome() else {
            panic!("an allow answers game-profile-request");
        };
        assert_eq!(result.denied, None);
        assert_eq!(result.profile.username, "Steve");

        let mut denied = profile_request(None);
        denied.deny("Closed");
        let EventOutcome::GameProfileRequest(result) = denied.into_outcome() else {
            panic!("a deny answers game-profile-request");
        };
        assert_eq!(result.denied, Some(Component::text("Closed").to_arena()));
    }

    #[test]
    fn a_profile_edit_carries_the_deny_it_saw() {
        let banned = Component::text("Banned");
        let mut renamed = profile_request(Some(banned.to_arena()));
        renamed.profile_mut().username = "Alex".into();
        let EventOutcome::GameProfileRequest(result) = renamed.into_outcome() else {
            panic!("a profile edit answers game-profile-request");
        };
        assert_eq!(result.profile.username, "Alex");
        assert_eq!(result.denied, Some(banned.to_arena()));
    }
}
