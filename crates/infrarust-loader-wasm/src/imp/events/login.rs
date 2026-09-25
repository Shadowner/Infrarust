use infrarust_api::event::ResultedEvent;
use infrarust_api::events::lifecycle::{
    DisconnectCause, DisconnectEvent, GameProfileRequestEvent, LoginEvent, LoginResult,
    OnlineAuthFailed, PermissionsSetupEvent, PermissionsSetupResult, PostLoginEvent, PreLoginEvent,
    PreLoginResult,
};

use super::{Applied, Texts, WasmEvent, unmatched};
use crate::bindings::infrarust::plugin::events::{self as we, EventKind};
use crate::component;
use crate::convert;

impl WasmEvent for PreLoginEvent {
    const KIND: EventKind = EventKind::PreLogin;

    fn to_wit(&self) -> we::Event {
        we::Event::PreLogin(we::PreLoginEvent {
            profile: convert::game_profile_to_wit(&self.profile),
            remote_addr: convert::socket_to_wit(self.remote_addr),
            protocol: self.protocol_version.raw(),
            server_domain: self.server_domain.clone(),
            result: match self.result() {
                PreLoginResult::Denied { reason } => {
                    we::PreLoginResult::Denied(component::to_wit(reason))
                }
                PreLoginResult::ForceOffline => we::PreLoginResult::ForceOffline,
                PreLoginResult::ForceOnline => we::PreLoginResult::ForceOnline,
                _ => we::PreLoginResult::Allowed,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::PreLogin(result) = outcome else {
            return unmatched(&outcome);
        };
        let mut texts = Texts::default();
        self.set_result(match result {
            we::PreLoginResult::Allowed => PreLoginResult::Allowed,
            we::PreLoginResult::Denied(reason) => PreLoginResult::Denied {
                reason: texts.convert(&reason),
            },
            we::PreLoginResult::ForceOffline => PreLoginResult::ForceOffline,
            we::PreLoginResult::ForceOnline => PreLoginResult::ForceOnline,
        });
        texts.applied()
    }
}

impl WasmEvent for PostLoginEvent {
    const KIND: EventKind = EventKind::PostLogin;

    fn to_wit(&self) -> we::Event {
        we::Event::PostLogin(we::PostLoginEvent {
            player: convert::player_ref(&*self.player),
            profile: convert::game_profile_to_wit(&self.profile),
            protocol: self.protocol_version.raw(),
        })
    }
}

impl WasmEvent for DisconnectEvent {
    const KIND: EventKind = EventKind::Disconnect;

    fn to_wit(&self) -> we::Event {
        let reason = |reason: &Option<_>| reason.as_ref().map(component::to_wit);
        we::Event::Disconnect(we::DisconnectEvent {
            player: convert::player_ref(&*self.player),
            last_server: self.last_server.as_ref().map(|s| s.as_str().to_owned()),
            cause: match &self.cause {
                DisconnectCause::ClientQuit => we::DisconnectCause::ClientQuit,
                DisconnectCause::Kicked { reason: text } => {
                    we::DisconnectCause::Kicked(reason(text))
                }
                DisconnectCause::BackendClosed { reason: text } => {
                    we::DisconnectCause::BackendClosed(reason(text))
                }
                DisconnectCause::Shutdown => we::DisconnectCause::Shutdown,
                _ => we::DisconnectCause::Error,
            },
        })
    }
}

impl WasmEvent for OnlineAuthFailed {
    const KIND: EventKind = EventKind::OnlineAuthFailed;

    fn to_wit(&self) -> we::Event {
        we::Event::OnlineAuthFailed(we::OnlineAuthFailedEvent {
            username: self.username.clone(),
        })
    }
}

impl WasmEvent for PermissionsSetupEvent {
    const KIND: EventKind = EventKind::PermissionsSetup;

    fn to_wit(&self) -> we::Event {
        we::Event::PermissionsSetup(we::PermissionsSetupEvent {
            player: convert::player_ref(&*self.player),
            online_mode: self.online_mode,
            result: we::PermissionsSetupResult::UseDefault,
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::PermissionsSetup(result) = outcome else {
            return unmatched(&outcome);
        };
        self.set_result(match result {
            we::PermissionsSetupResult::UseDefault => PermissionsSetupResult::UseDefault,
        });
        Applied::Set
    }
}

impl WasmEvent for LoginEvent {
    const KIND: EventKind = EventKind::Login;

    fn to_wit(&self) -> we::Event {
        we::Event::Login(we::LoginEvent {
            player: convert::player_ref(&*self.player),
            online_mode: self.online_mode,
            result: match self.result() {
                LoginResult::Denied { reason } => {
                    we::LoginResult::Denied(component::to_wit(reason))
                }
                _ => we::LoginResult::Allowed,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::Login(result) = outcome else {
            return unmatched(&outcome);
        };
        let mut texts = Texts::default();
        self.set_result(match result {
            we::LoginResult::Allowed => LoginResult::Allowed,
            we::LoginResult::Denied(reason) => LoginResult::Denied {
                reason: texts.convert(&reason),
            },
        });
        texts.applied()
    }
}

impl WasmEvent for GameProfileRequestEvent {
    const KIND: EventKind = EventKind::GameProfileRequest;

    fn to_wit(&self) -> we::Event {
        we::Event::GameProfileRequest(we::GameProfileRequestEvent {
            original: convert::game_profile_to_wit(self.original()),
            online_mode: self.online_mode,
            remote_addr: convert::socket_to_wit(self.remote_addr),
            virtual_host: self.virtual_host.clone(),
            protocol: self.protocol_version.raw(),
            result: we::GameProfileRequestResult {
                profile: convert::game_profile_to_wit(&self.profile),
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::GameProfileRequest(result) = outcome else {
            return unmatched(&outcome);
        };
        self.profile = convert::game_profile_from_wit(result.profile);
        Applied::Set
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use infrarust_api::permissions::DefaultPermissionChecker;
    use infrarust_api::types::{Component, ProtocolVersion, ServerId};

    use super::super::steve;
    use super::*;

    fn pre_login() -> PreLoginEvent {
        PreLoginEvent::new(
            steve().profile().clone(),
            "203.0.113.7:51234".parse().unwrap(),
            ProtocolVersion::new(767),
            "play.example.com".into(),
        )
    }

    fn current(event: &PreLoginEvent) -> we::PreLoginResult {
        let we::Event::PreLogin(record) = event.to_wit() else {
            panic!("a pre-login event is sent as pre-login");
        };
        record.result
    }

    #[test]
    fn the_guest_sees_the_current_result() {
        let mut event = pre_login();
        assert_eq!(current(&event), we::PreLoginResult::Allowed);
        event.deny(Component::text("Banned"));
        assert_eq!(
            current(&event),
            we::PreLoginResult::Denied(component::to_wit(&Component::text("Banned")))
        );
    }

    #[test]
    fn unchanged_keeps_and_allowed_resets_an_earlier_deny() {
        let mut event = pre_login();
        event.deny(Component::text("Banned"));
        assert_eq!(event.apply(we::EventOutcome::Unchanged), Applied::Unchanged);
        assert!(matches!(event.result(), PreLoginResult::Denied { .. }));

        let reset = we::EventOutcome::PreLogin(we::PreLoginResult::Allowed);
        assert_eq!(event.apply(reset), Applied::Set);
        assert!(matches!(event.result(), PreLoginResult::Allowed));
    }

    #[test]
    fn a_deny_with_an_invalid_reason_still_denies_with_the_fallback() {
        let mut event = pre_login();
        let broken = crate::bindings::infrarust::plugin::types::Component { nodes: vec![] };
        let applied = event.apply(we::EventOutcome::PreLogin(we::PreLoginResult::Denied(
            broken,
        )));
        assert!(matches!(applied, Applied::Fallback(_)), "{applied:?}");
        match event.result() {
            PreLoginResult::Denied { reason } => assert_eq!(*reason, component::fallback()),
            other => panic!("expected a deny, got {other:?}"),
        }
    }

    #[test]
    fn another_events_outcome_is_ignored() {
        let mut event = pre_login();
        let applied = event.apply(we::EventOutcome::ChatMessage(we::ChatMessageResult::Allow));
        assert_eq!(applied, Applied::Mismatched);
        assert!(matches!(event.result(), PreLoginResult::Allowed));
    }

    #[test]
    fn a_disconnect_carries_its_cause_and_reason() {
        let event = DisconnectEvent::new(
            steve(),
            Some(ServerId::new("lobby")),
            DisconnectCause::Kicked {
                reason: Some(Component::text("bye")),
            },
        );
        let we::Event::Disconnect(record) = event.to_wit() else {
            panic!("a disconnect is sent as disconnect");
        };
        assert_eq!(record.player.id, 1);
        assert_eq!(record.last_server.as_deref(), Some("lobby"));
        assert_eq!(
            record.cause,
            we::DisconnectCause::Kicked(Some(component::to_wit(&Component::text("bye"))))
        );
    }

    #[test]
    fn a_login_deny_sets_the_native_result_and_allowed_resets_it() {
        let mut event = LoginEvent::new(steve(), true);
        let reason = component::to_wit(&Component::text("closed"));
        assert_eq!(
            event.apply(we::EventOutcome::Login(we::LoginResult::Denied(
                reason.clone()
            ))),
            Applied::Set
        );
        assert!(
            matches!(event.result(), LoginResult::Denied { reason } if *reason == Component::text("closed"))
        );
        let we::Event::Login(record) = event.to_wit() else {
            panic!("a login is sent as login");
        };
        assert_eq!(record.result, we::LoginResult::Denied(reason));
        event.apply(we::EventOutcome::Login(we::LoginResult::Allowed));
        assert!(matches!(event.result(), LoginResult::Allowed));
    }

    #[test]
    fn a_profile_outcome_replaces_the_profile_and_keeps_the_original() {
        let profile = steve().profile().clone();
        let mut event = GameProfileRequestEvent::new(
            profile.clone(),
            false,
            "203.0.113.7:51234".parse().unwrap(),
            Some("play.example.com".into()),
            ProtocolVersion::new(767),
        );
        let we::Event::GameProfileRequest(record) = event.to_wit() else {
            panic!("a profile request is sent as game-profile-request");
        };
        assert_eq!(record.original, record.result.profile);
        let mut renamed = record.result.profile.clone();
        renamed.username = "Alex".into();
        assert_eq!(
            event.apply(we::EventOutcome::GameProfileRequest(
                we::GameProfileRequestResult { profile: renamed }
            )),
            Applied::Set
        );
        assert_eq!(event.profile.username, "Alex");
        assert_eq!(event.original(), &profile);
        assert!(event.is_modified());
    }

    #[test]
    fn use_default_replaces_a_custom_checker() {
        let mut event = PermissionsSetupEvent::new(steve(), true);
        event.set_result(PermissionsSetupResult::Custom(Arc::new(
            DefaultPermissionChecker,
        )));
        let applied = event.apply(we::EventOutcome::PermissionsSetup(
            we::PermissionsSetupResult::UseDefault,
        ));
        assert_eq!(applied, Applied::Set);
        assert!(matches!(event.result(), PermissionsSetupResult::UseDefault));
    }
}
