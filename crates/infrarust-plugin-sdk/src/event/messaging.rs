use super::ResultCell;
use crate::bindings::events as we;
use crate::types::{ChannelId, FromWit, PlayerRef, ServerId};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MessageEndpoint {
    Client,
    Backend(ServerId),
}

pub use infrarust_plugin_common::enums::MessagePhase;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PluginMessageResult {
    Forward,
    Handled,
    Replace(Vec<u8>),
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PluginMessageEvent {
    pub player: PlayerRef,
    pub source: MessageEndpoint,
    pub channel: ChannelId,
    pub raw_channel: String,
    pub data: Vec<u8>,
    pub phase: MessagePhase,
    result: ResultCell<PluginMessageResult>,
}

impl PluginMessageEvent {
    #[must_use]
    pub fn from_client(&self) -> bool {
        self.source == MessageEndpoint::Client
    }

    #[must_use]
    pub const fn result(&self) -> &PluginMessageResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: PluginMessageResult) {
        self.result.set(result);
    }

    pub fn forward(&mut self) {
        self.set_result(PluginMessageResult::Forward);
    }

    pub fn handled(&mut self) {
        self.set_result(PluginMessageResult::Handled);
    }

    pub fn replace(&mut self, data: impl Into<Vec<u8>>) {
        self.set_result(PluginMessageResult::Replace(data.into()));
    }
}

guest_event!(
    PluginMessageEvent,
    PluginMessage,
    |e| Self {
        player: PlayerRef::from_wit(e.player),
        source: match e.source {
            we::MessageEndpoint::Client => MessageEndpoint::Client,
            we::MessageEndpoint::Backend(server) => {
                MessageEndpoint::Backend(ServerId::from(server))
            }
        },
        channel: ChannelId::from_wit(e.channel),
        raw_channel: e.raw_channel,
        data: e.data,
        phase: MessagePhase::from_wit(e.phase),
        result: ResultCell::new(match e.result {
            we::PluginMessageResult::Forward => PluginMessageResult::Forward,
            we::PluginMessageResult::Handled => PluginMessageResult::Handled,
            we::PluginMessageResult::Replace(data) => PluginMessageResult::Replace(data),
        }),
    },
    result,
    |r| match r {
        PluginMessageResult::Forward => we::PluginMessageResult::Forward,
        PluginMessageResult::Handled => we::PluginMessageResult::Handled,
        PluginMessageResult::Replace(data) => we::PluginMessageResult::Replace(data),
    }
);

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoginPluginRequestResult {
    Forward,
    Replace(Vec<u8>),
    Respond(Option<Vec<u8>>),
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LoginPluginRequestEvent {
    pub player: PlayerRef,
    pub server: ServerId,
    pub message_id: i32,
    pub channel: String,
    pub data: Vec<u8>,
    pub client_reachable: bool,
    result: ResultCell<LoginPluginRequestResult>,
}

impl LoginPluginRequestEvent {
    #[must_use]
    pub const fn result(&self) -> &LoginPluginRequestResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: LoginPluginRequestResult) {
        self.result.set(result);
    }

    pub fn forward(&mut self) {
        self.set_result(LoginPluginRequestResult::Forward);
    }

    pub fn replace(&mut self, data: impl Into<Vec<u8>>) {
        self.set_result(LoginPluginRequestResult::Replace(data.into()));
    }

    pub fn respond(&mut self, data: impl Into<Vec<u8>>) {
        self.set_result(LoginPluginRequestResult::Respond(Some(data.into())));
    }

    pub fn respond_not_understood(&mut self) {
        self.set_result(LoginPluginRequestResult::Respond(None));
    }
}

guest_event!(
    LoginPluginRequestEvent,
    LoginPluginRequest,
    |e| Self {
        player: PlayerRef::from_wit(e.player),
        server: ServerId::from(e.server),
        message_id: e.message_id,
        channel: e.channel,
        data: e.data,
        client_reachable: e.client_reachable,
        result: ResultCell::new(match e.result {
            we::LoginPluginRequestResult::Forward => LoginPluginRequestResult::Forward,
            we::LoginPluginRequestResult::Replace(data) => LoginPluginRequestResult::Replace(data),
            we::LoginPluginRequestResult::Respond(data) => LoginPluginRequestResult::Respond(data),
        }),
    },
    result,
    |r| match r {
        LoginPluginRequestResult::Forward => we::LoginPluginRequestResult::Forward,
        LoginPluginRequestResult::Replace(data) => we::LoginPluginRequestResult::Replace(data),
        LoginPluginRequestResult::Respond(data) => we::LoginPluginRequestResult::Respond(data),
    }
);

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoginPluginResponseResult {
    Forward,
    Replace { successful: bool, data: Vec<u8> },
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LoginPluginResponseEvent {
    pub player: PlayerRef,
    pub server: ServerId,
    pub message_id: i32,
    pub channel: String,
    pub successful: bool,
    pub data: Vec<u8>,
    result: ResultCell<LoginPluginResponseResult>,
}

impl LoginPluginResponseEvent {
    #[must_use]
    pub const fn result(&self) -> &LoginPluginResponseResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: LoginPluginResponseResult) {
        self.result.set(result);
    }

    pub fn forward(&mut self) {
        self.set_result(LoginPluginResponseResult::Forward);
    }

    pub fn replace(&mut self, successful: bool, data: impl Into<Vec<u8>>) {
        self.set_result(LoginPluginResponseResult::Replace {
            successful,
            data: data.into(),
        });
    }
}

guest_event!(
    LoginPluginResponseEvent,
    LoginPluginResponse,
    |e| Self {
        player: PlayerRef::from_wit(e.player),
        server: ServerId::from(e.server),
        message_id: e.message_id,
        channel: e.channel,
        successful: e.successful,
        data: e.data,
        result: ResultCell::new(match e.result {
            we::LoginPluginResponseResult::Forward => LoginPluginResponseResult::Forward,
            we::LoginPluginResponseResult::Replace(replacement) => {
                LoginPluginResponseResult::Replace {
                    successful: replacement.successful,
                    data: replacement.data,
                }
            }
        }),
    },
    result,
    |r| match r {
        LoginPluginResponseResult::Forward => we::LoginPluginResponseResult::Forward,
        LoginPluginResponseResult::Replace { successful, data } => {
            we::LoginPluginResponseResult::Replace(we::LoginPluginResponseReplacement {
                successful,
                data,
            })
        }
    }
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedResponse {
    pub content_type: String,
    pub payload: Vec<u8>,
}

impl NamedResponse {
    #[must_use]
    pub fn new(content_type: impl Into<String>, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            content_type: content_type.into(),
            payload: payload.into(),
        }
    }

    #[must_use]
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.payload).ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct NamedOutcome {
    pub cancelled: bool,
    pub response: Option<NamedResponse>,
}

impl NamedOutcome {
    pub(crate) fn from_wit(result: we::NamedEventResult) -> Self {
        Self {
            cancelled: result.cancelled,
            response: result
                .response
                .map(|response| NamedResponse::new(response.content_type, response.payload)),
        }
    }

    fn to_wit(&self) -> we::NamedEventResult {
        we::NamedEventResult {
            cancelled: self.cancelled,
            response: self
                .response
                .as_ref()
                .map(|response| we::NamedEventResponse {
                    content_type: response.content_type.clone(),
                    payload: response.payload.clone(),
                }),
        }
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NamedEvent {
    pub name: String,
    pub source_plugin: String,
    pub content_type: String,
    pub payload: Vec<u8>,
    outcome: ResultCell<NamedOutcome>,
}

impl NamedEvent {
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.payload).ok()
    }

    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        self.outcome.get().cancelled
    }

    #[must_use]
    pub const fn response(&self) -> Option<&NamedResponse> {
        self.outcome.get().response.as_ref()
    }

    pub fn cancel(&mut self) {
        self.outcome.get_mut().cancelled = true;
    }

    pub fn uncancel(&mut self) {
        self.outcome.get_mut().cancelled = false;
    }

    pub fn respond(&mut self, content_type: impl Into<String>, payload: impl Into<Vec<u8>>) {
        self.outcome.get_mut().response = Some(NamedResponse::new(content_type, payload));
    }

    pub fn respond_text(&mut self, text: impl Into<String>) {
        self.respond("text/plain", text.into().into_bytes());
    }

    pub fn clear_response(&mut self) {
        self.outcome.get_mut().response = None;
    }
}

guest_event!(
    NamedEvent,
    NamedEvent,
    |e| Self {
        name: e.name,
        source_plugin: e.source_plugin,
        content_type: e.content_type,
        payload: e.payload,
        outcome: ResultCell::new(NamedOutcome::from_wit(e.result)),
    },
    outcome,
    |outcome| outcome.to_wit()
);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::bindings::events::{Event, EventOutcome};
    use crate::event::GuestEvent;

    fn named(result: we::NamedEventResult) -> NamedEvent {
        NamedEvent::from_event(Event::NamedEvent(we::NamedEventEvent {
            name: "echo".into(),
            source_plugin: "relay".into(),
            content_type: "text/plain".into(),
            payload: b"hi".to_vec(),
            result,
        }))
        .unwrap()
    }

    fn steve() -> crate::bindings::types::PlayerRef {
        crate::bindings::types::PlayerRef {
            id: 7,
            uuid: crate::bindings::types::Uuid { hi: 0, lo: 7 },
            username: "Steve".into(),
        }
    }

    fn request(result: we::LoginPluginRequestResult) -> LoginPluginRequestEvent {
        LoginPluginRequestEvent::from_event(Event::LoginPluginRequest(
            we::LoginPluginRequestEvent {
                player: steve(),
                server: "modded".into(),
                message_id: 4,
                channel: "automodpack:handshake".into(),
                data: b"\x01".to_vec(),
                client_reachable: true,
                result,
            },
        ))
        .unwrap()
    }

    #[test]
    fn a_login_plugin_request_answers_with_each_helper() {
        let event = request(we::LoginPluginRequestResult::Forward);
        assert_eq!(event.player.username, "Steve");
        assert_eq!(event.server, ServerId::new("modded"));
        assert_eq!(event.message_id, 4);
        assert!(event.client_reachable);
        assert_eq!(event.into_outcome(), EventOutcome::Unchanged);

        let mut event = request(we::LoginPluginRequestResult::Forward);
        event.respond(b"ok".to_vec());
        assert_eq!(
            event.into_outcome(),
            EventOutcome::LoginPluginRequest(we::LoginPluginRequestResult::Respond(Some(
                b"ok".to_vec()
            )))
        );

        let mut event = request(we::LoginPluginRequestResult::Forward);
        event.respond_not_understood();
        assert_eq!(
            event.into_outcome(),
            EventOutcome::LoginPluginRequest(we::LoginPluginRequestResult::Respond(None))
        );

        let mut event = request(we::LoginPluginRequestResult::Respond(None));
        assert_eq!(event.result(), &LoginPluginRequestResult::Respond(None));
        event.replace(b"new".to_vec());
        assert_eq!(
            event.into_outcome(),
            EventOutcome::LoginPluginRequest(we::LoginPluginRequestResult::Replace(
                b"new".to_vec()
            ))
        );

        let mut event = request(we::LoginPluginRequestResult::Replace(b"x".to_vec()));
        event.forward();
        assert_eq!(
            event.into_outcome(),
            EventOutcome::LoginPluginRequest(we::LoginPluginRequestResult::Forward)
        );
    }

    #[test]
    fn a_login_plugin_response_can_be_replaced() {
        let mut event = LoginPluginResponseEvent::from_event(Event::LoginPluginResponse(
            we::LoginPluginResponseEvent {
                player: steve(),
                server: "modded".into(),
                message_id: 4,
                channel: "automodpack:handshake".into(),
                successful: true,
                data: b"\x02".to_vec(),
                result: we::LoginPluginResponseResult::Forward,
            },
        ))
        .unwrap();
        assert!(event.successful);
        assert_eq!(event.channel, "automodpack:handshake");
        event.replace(false, Vec::new());
        assert_eq!(
            event.into_outcome(),
            EventOutcome::LoginPluginResponse(we::LoginPluginResponseResult::Replace(
                we::LoginPluginResponseReplacement {
                    successful: false,
                    data: Vec::new(),
                }
            ))
        );
    }

    #[test]
    fn an_earlier_answer_is_visible_and_kept_when_untouched() {
        let earlier = we::NamedEventResult {
            cancelled: true,
            response: Some(we::NamedEventResponse {
                content_type: "text/plain".into(),
                payload: b"pong".to_vec(),
            }),
        };
        let event = named(earlier);
        assert!(event.is_cancelled());
        assert_eq!(event.response().and_then(NamedResponse::text), Some("pong"));
        assert_eq!(event.text(), Some("hi"));
        assert_eq!(event.into_outcome(), EventOutcome::Unchanged);
    }

    #[test]
    fn responding_sends_the_whole_answer_back() {
        let mut event = named(we::NamedEventResult {
            cancelled: true,
            response: None,
        });
        event.respond_text("pong");
        assert_eq!(
            event.into_outcome(),
            EventOutcome::NamedEvent(we::NamedEventResult {
                cancelled: true,
                response: Some(we::NamedEventResponse {
                    content_type: "text/plain".into(),
                    payload: b"pong".to_vec()
                })
            })
        );
    }
}
