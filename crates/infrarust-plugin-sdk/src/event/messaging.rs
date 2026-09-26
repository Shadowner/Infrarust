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
