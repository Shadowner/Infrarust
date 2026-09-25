use bytes::Bytes;

use crate::event::Event;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NamedEvent {
    pub name: String,
    pub source_plugin: String,
    pub content_type: String,
    pub payload: Bytes,
    pub cancelled: bool,
    pub response: Option<NamedEventResponse>,
}

impl NamedEvent {
    pub fn new(
        name: impl Into<String>,
        content_type: impl Into<String>,
        payload: impl Into<Bytes>,
    ) -> Self {
        Self {
            name: name.into(),
            source_plugin: String::new(),
            content_type: content_type.into(),
            payload: payload.into(),
            cancelled: false,
            response: None,
        }
    }

    pub const fn cancel(&mut self) {
        self.cancelled = true;
    }

    pub fn respond(&mut self, content_type: impl Into<String>, payload: impl Into<Bytes>) {
        self.response = Some(NamedEventResponse::new(content_type, payload));
    }
}

impl Event for NamedEvent {}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NamedEventResponse {
    pub content_type: String,
    pub payload: Bytes,
}

impl NamedEventResponse {
    pub fn new(content_type: impl Into<String>, payload: impl Into<Bytes>) -> Self {
        Self {
            content_type: content_type.into(),
            payload: payload.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_named_event_is_open_and_unanswered() {
        let event = NamedEvent::new("chat:relay", "application/json", &b"{}"[..]);

        assert_eq!(event.name, "chat:relay");
        assert_eq!(event.content_type, "application/json");
        assert_eq!(event.payload, Bytes::from_static(b"{}"));
        assert!(event.source_plugin.is_empty());
        assert!(!event.cancelled);
        assert_eq!(event.response, None);
    }

    #[test]
    fn respond_and_cancel_record_the_listener_outcome() {
        let mut event = NamedEvent::new("ping", "text/plain", "hi");

        event.respond("text/plain", "pong");
        event.cancel();

        assert!(event.cancelled);
        assert_eq!(
            event.response,
            Some(NamedEventResponse::new("text/plain", "pong"))
        );
    }
}
