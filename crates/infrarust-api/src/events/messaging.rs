use std::sync::Arc;

use bytes::Bytes;

use crate::messaging::{ChannelId, Endpoint, MessagePhase};
use crate::player::Player;
use crate::types::ServerId;

#[non_exhaustive]
pub struct PluginMessageEvent {
    pub player: Arc<dyn Player>,
    pub source: Endpoint,
    pub channel: ChannelId,
    pub raw_channel: String,
    pub data: Bytes,
    pub phase: MessagePhase,
    result: PluginMessageResult,
}

impl PluginMessageEvent {
    pub fn new(
        player: Arc<dyn Player>,
        source: Endpoint,
        channel: ChannelId,
        raw_channel: String,
        data: Bytes,
        phase: MessagePhase,
    ) -> Self {
        Self {
            player,
            source,
            channel,
            raw_channel,
            data,
            phase,
            result: PluginMessageResult::default(),
        }
    }

    pub fn from_client(&self) -> bool {
        self.source == Endpoint::Client
    }

    pub fn forward(&mut self) {
        self.result = PluginMessageResult::Forward;
    }

    pub fn handled(&mut self) {
        self.result = PluginMessageResult::Handled;
    }

    pub fn replace(&mut self, data: impl Into<Bytes>) {
        self.result = PluginMessageResult::Replace(data.into());
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PluginMessageResult {
    #[default]
    Forward,
    Handled,
    Replace(Bytes),
}

crate::events::player_event!(PluginMessageEvent);

crate::event::resulted_event!(PluginMessageEvent, PluginMessageResult);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoginPluginRequestResult {
    #[default]
    Forward,
    Replace(Bytes),
    Respond(Option<Bytes>),
}

#[non_exhaustive]
pub struct LoginPluginRequestEvent {
    pub player: Arc<dyn Player>,
    pub server: ServerId,
    pub message_id: i32,
    pub channel: String,
    pub data: Bytes,
    pub client_reachable: bool,
    result: LoginPluginRequestResult,
}

impl LoginPluginRequestEvent {
    pub fn new(
        player: Arc<dyn Player>,
        server: ServerId,
        message_id: i32,
        channel: impl Into<String>,
        data: impl Into<Bytes>,
        client_reachable: bool,
    ) -> Self {
        Self {
            player,
            server,
            message_id,
            channel: channel.into(),
            data: data.into(),
            client_reachable,
            result: LoginPluginRequestResult::Forward,
        }
    }

    pub fn forward(&mut self) {
        self.result = LoginPluginRequestResult::Forward;
    }

    pub fn replace(&mut self, data: impl Into<Bytes>) {
        self.result = LoginPluginRequestResult::Replace(data.into());
    }

    pub fn respond(&mut self, data: impl Into<Bytes>) {
        self.result = LoginPluginRequestResult::Respond(Some(data.into()));
    }

    pub fn respond_not_understood(&mut self) {
        self.result = LoginPluginRequestResult::Respond(None);
    }
}

crate::events::player_event!(LoginPluginRequestEvent);

crate::event::resulted_event!(LoginPluginRequestEvent, LoginPluginRequestResult);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoginPluginResponseResult {
    #[default]
    Forward,
    Replace {
        successful: bool,
        data: Bytes,
    },
}

#[non_exhaustive]
pub struct LoginPluginResponseEvent {
    pub player: Arc<dyn Player>,
    pub server: ServerId,
    pub message_id: i32,
    pub channel: String,
    pub successful: bool,
    pub data: Bytes,
    result: LoginPluginResponseResult,
}

impl LoginPluginResponseEvent {
    pub fn new(
        player: Arc<dyn Player>,
        server: ServerId,
        message_id: i32,
        channel: impl Into<String>,
        successful: bool,
        data: impl Into<Bytes>,
    ) -> Self {
        Self {
            player,
            server,
            message_id,
            channel: channel.into(),
            successful,
            data: data.into(),
            result: LoginPluginResponseResult::Forward,
        }
    }

    pub fn forward(&mut self) {
        self.result = LoginPluginResponseResult::Forward;
    }

    pub fn replace(&mut self, successful: bool, data: impl Into<Bytes>) {
        self.result = LoginPluginResponseResult::Replace {
            successful,
            data: data.into(),
        };
    }
}

crate::events::player_event!(LoginPluginResponseEvent);

crate::event::resulted_event!(LoginPluginResponseEvent, LoginPluginResponseResult);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::event::ResultedEvent;
    use crate::test_util::MockPlayer;
    use crate::types::PlayerId;

    fn steve() -> Arc<dyn Player> {
        MockPlayer::new(7, "Steve").into_arc()
    }

    #[test]
    fn a_login_plugin_request_is_forwarded_until_a_listener_decides() {
        let mut event = LoginPluginRequestEvent::new(
            steve(),
            ServerId::new("modded"),
            3,
            "fml:loginwrapper",
            &b"\x01"[..],
            true,
        );
        assert_eq!(event.result(), &LoginPluginRequestResult::Forward);
        assert_eq!(event.player_id(), PlayerId::new(7));
        assert_eq!(event.message_id, 3);
        assert_eq!(event.channel, "fml:loginwrapper");
        assert!(event.client_reachable);

        event.replace(&b"new"[..]);
        assert_eq!(
            event.result(),
            &LoginPluginRequestResult::Replace(Bytes::from_static(b"new"))
        );
        event.respond(&b"ok"[..]);
        assert_eq!(
            event.result(),
            &LoginPluginRequestResult::Respond(Some(Bytes::from_static(b"ok")))
        );
        event.respond_not_understood();
        assert_eq!(event.result(), &LoginPluginRequestResult::Respond(None));
        event.forward();
        assert_eq!(event.result(), &LoginPluginRequestResult::Forward);
    }

    #[test]
    fn a_login_plugin_response_carries_the_request_channel() {
        let mut event = LoginPluginResponseEvent::new(
            steve(),
            ServerId::new("modded"),
            3,
            "automodpack:handshake",
            true,
            &b"\x00"[..],
        );
        assert_eq!(event.result(), &LoginPluginResponseResult::Forward);
        assert_eq!(event.channel, "automodpack:handshake");
        assert!(event.successful);

        event.replace(false, Bytes::new());
        assert_eq!(
            event.result(),
            &LoginPluginResponseResult::Replace {
                successful: false,
                data: Bytes::new()
            }
        );
        event.forward();
        assert_eq!(event.result(), &LoginPluginResponseResult::Forward);
    }
}
