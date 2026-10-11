use bytes::Bytes;
use infrarust_api::event::ResultedEvent;
use infrarust_api::events::messaging::{
    LoginPluginRequestEvent, LoginPluginRequestResult, LoginPluginResponseEvent,
    LoginPluginResponseResult, PluginMessageEvent, PluginMessageResult,
};
use infrarust_api::events::named::{NamedEvent, NamedEventResponse};
use infrarust_api::messaging::{Endpoint, MessagePhase};

use super::{Applied, WasmEvent, unmatched};
use crate::bindings::infrarust::plugin::events::{self as we, EventKind};
use crate::convert;

impl WasmEvent for LoginPluginRequestEvent {
    const KIND: EventKind = EventKind::LoginPluginRequest;

    fn to_wit(&self) -> we::Event {
        we::Event::LoginPluginRequest(we::LoginPluginRequestEvent {
            player: convert::player_ref(&*self.player),
            server: self.server.as_str().to_owned(),
            message_id: self.message_id,
            channel: self.channel.clone(),
            data: self.data.to_vec(),
            client_reachable: self.client_reachable,
            result: match self.result() {
                LoginPluginRequestResult::Replace(data) => {
                    we::LoginPluginRequestResult::Replace(data.to_vec())
                }
                LoginPluginRequestResult::Respond(data) => {
                    we::LoginPluginRequestResult::Respond(data.as_ref().map(|data| data.to_vec()))
                }
                _ => we::LoginPluginRequestResult::Forward,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::LoginPluginRequest(result) = outcome else {
            return unmatched(&outcome);
        };
        self.set_result(match result {
            we::LoginPluginRequestResult::Forward => LoginPluginRequestResult::Forward,
            we::LoginPluginRequestResult::Replace(data) => {
                LoginPluginRequestResult::Replace(Bytes::from(data))
            }
            we::LoginPluginRequestResult::Respond(data) => {
                LoginPluginRequestResult::Respond(data.map(Bytes::from))
            }
        });
        Applied::Set
    }
}

impl WasmEvent for LoginPluginResponseEvent {
    const KIND: EventKind = EventKind::LoginPluginResponse;

    fn to_wit(&self) -> we::Event {
        we::Event::LoginPluginResponse(we::LoginPluginResponseEvent {
            player: convert::player_ref(&*self.player),
            server: self.server.as_str().to_owned(),
            message_id: self.message_id,
            channel: self.channel.clone(),
            successful: self.successful,
            data: self.data.to_vec(),
            result: match self.result() {
                LoginPluginResponseResult::Replace { successful, data } => {
                    we::LoginPluginResponseResult::Replace(we::LoginPluginResponseReplacement {
                        successful: *successful,
                        data: data.to_vec(),
                    })
                }
                _ => we::LoginPluginResponseResult::Forward,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::LoginPluginResponse(result) = outcome else {
            return unmatched(&outcome);
        };
        self.set_result(match result {
            we::LoginPluginResponseResult::Forward => LoginPluginResponseResult::Forward,
            we::LoginPluginResponseResult::Replace(replacement) => {
                LoginPluginResponseResult::Replace {
                    successful: replacement.successful,
                    data: Bytes::from(replacement.data),
                }
            }
        });
        Applied::Set
    }
}

fn endpoint(source: &Endpoint) -> we::MessageEndpoint {
    match source {
        Endpoint::Backend(server) => we::MessageEndpoint::Backend(server.as_str().to_owned()),
        _ => we::MessageEndpoint::Client,
    }
}

impl WasmEvent for PluginMessageEvent {
    const KIND: EventKind = EventKind::PluginMessage;

    fn to_wit(&self) -> we::Event {
        we::Event::PluginMessage(we::PluginMessageEvent {
            player: convert::player_ref(&*self.player),
            source: endpoint(&self.source),
            channel: convert::channel_to_wit(&self.channel),
            raw_channel: self.raw_channel.clone(),
            data: self.data.to_vec(),
            phase: match self.phase {
                MessagePhase::Configuration => we::MessagePhase::Configuration,
                _ => we::MessagePhase::Play,
            },
            result: match self.result() {
                PluginMessageResult::Handled => we::PluginMessageResult::Handled,
                PluginMessageResult::Replace(data) => {
                    we::PluginMessageResult::Replace(data.to_vec())
                }
                _ => we::PluginMessageResult::Forward,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::PluginMessage(result) = outcome else {
            return unmatched(&outcome);
        };
        self.set_result(match result {
            we::PluginMessageResult::Forward => PluginMessageResult::Forward,
            we::PluginMessageResult::Handled => PluginMessageResult::Handled,
            we::PluginMessageResult::Replace(data) => {
                PluginMessageResult::Replace(Bytes::from(data))
            }
        });
        Applied::Set
    }
}

pub(crate) fn named_result(event: &NamedEvent) -> we::NamedEventResult {
    we::NamedEventResult {
        cancelled: event.cancelled,
        response: event
            .response
            .as_ref()
            .map(|response| we::NamedEventResponse {
                content_type: response.content_type.clone(),
                payload: response.payload.to_vec(),
            }),
    }
}

impl WasmEvent for NamedEvent {
    const KIND: EventKind = EventKind::NamedEvent;

    fn to_wit(&self) -> we::Event {
        we::Event::NamedEvent(we::NamedEventEvent {
            name: self.name.clone(),
            source_plugin: self.source_plugin.clone(),
            content_type: self.content_type.clone(),
            payload: self.payload.to_vec(),
            result: named_result(self),
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::NamedEvent(result) = outcome else {
            return unmatched(&outcome);
        };
        self.cancelled = result.cancelled;
        self.response = result
            .response
            .map(|response| NamedEventResponse::new(response.content_type, response.payload));
        Applied::Set
    }
}

#[cfg(test)]
mod tests {
    use infrarust_api::messaging::ChannelId;
    use infrarust_api::types::ServerId;

    use super::super::steve;
    use super::*;
    use crate::bindings::infrarust::plugin::types as wt;

    fn message() -> PluginMessageEvent {
        PluginMessageEvent::new(
            steve(),
            Endpoint::Backend(ServerId::new("lobby")),
            ChannelId::bungeecord(),
            "BungeeCord".into(),
            Bytes::from_static(b"\x00\x07Connect"),
            MessagePhase::Play,
        )
    }

    #[test]
    fn a_login_plugin_request_round_trips_every_answer() {
        let mut event = LoginPluginRequestEvent::new(
            steve(),
            ServerId::new("modded"),
            4,
            "automodpack:handshake",
            Bytes::from_static(b"\x01"),
            true,
        );
        let we::Event::LoginPluginRequest(record) = event.to_wit() else {
            panic!("a login plugin request is sent as login-plugin-request");
        };
        assert_eq!(record.server, "modded");
        assert_eq!(record.message_id, 4);
        assert_eq!(record.channel, "automodpack:handshake");
        assert_eq!(record.data, b"\x01");
        assert!(record.client_reachable);
        assert_eq!(record.result, we::LoginPluginRequestResult::Forward);

        for (answer, expected) in [
            (
                we::LoginPluginRequestResult::Replace(b"new".to_vec()),
                LoginPluginRequestResult::Replace(Bytes::from_static(b"new")),
            ),
            (
                we::LoginPluginRequestResult::Respond(Some(b"ok".to_vec())),
                LoginPluginRequestResult::Respond(Some(Bytes::from_static(b"ok"))),
            ),
            (
                we::LoginPluginRequestResult::Respond(None),
                LoginPluginRequestResult::Respond(None),
            ),
            (
                we::LoginPluginRequestResult::Forward,
                LoginPluginRequestResult::Forward,
            ),
        ] {
            assert_eq!(
                event.apply(we::EventOutcome::LoginPluginRequest(answer.clone())),
                Applied::Set
            );
            assert_eq!(event.result(), &expected);
            let we::Event::LoginPluginRequest(record) = event.to_wit() else {
                panic!("a login plugin request is sent as login-plugin-request");
            };
            assert_eq!(record.result, answer);
        }
        assert_eq!(
            event.apply(we::EventOutcome::PluginMessage(
                we::PluginMessageResult::Handled
            )),
            Applied::Mismatched
        );
    }

    #[test]
    fn a_login_plugin_response_can_be_replaced() {
        let mut event = LoginPluginResponseEvent::new(
            steve(),
            ServerId::new("modded"),
            4,
            "automodpack:handshake",
            true,
            Bytes::from_static(b"\x02"),
        );
        let we::Event::LoginPluginResponse(record) = event.to_wit() else {
            panic!("a login plugin response is sent as login-plugin-response");
        };
        assert!(record.successful);
        assert_eq!(record.channel, "automodpack:handshake");
        assert_eq!(record.result, we::LoginPluginResponseResult::Forward);

        let replacement =
            we::LoginPluginResponseResult::Replace(we::LoginPluginResponseReplacement {
                successful: false,
                data: Vec::new(),
            });
        assert_eq!(
            event.apply(we::EventOutcome::LoginPluginResponse(replacement.clone())),
            Applied::Set
        );
        assert_eq!(
            event.result(),
            &LoginPluginResponseResult::Replace {
                successful: false,
                data: Bytes::new()
            }
        );
        let we::Event::LoginPluginResponse(record) = event.to_wit() else {
            panic!("a login plugin response is sent as login-plugin-response");
        };
        assert_eq!(record.result, replacement);
        event.apply(we::EventOutcome::LoginPluginResponse(
            we::LoginPluginResponseResult::Forward,
        ));
        assert_eq!(event.result(), &LoginPluginResponseResult::Forward);
    }

    #[test]
    fn a_plugin_message_carries_both_channel_names_and_its_bytes() {
        let we::Event::PluginMessage(record) = message().to_wit() else {
            panic!("a plugin message is sent as plugin-message");
        };
        assert_eq!(
            record.channel,
            wt::ChannelId {
                modern: Some("bungeecord:main".into()),
                legacy: Some("BungeeCord".into())
            }
        );
        assert_eq!(record.raw_channel, "BungeeCord");
        assert_eq!(record.source, we::MessageEndpoint::Backend("lobby".into()));
        assert_eq!(record.data, b"\x00\x07Connect");
        assert_eq!(record.result, we::PluginMessageResult::Forward);
    }

    #[test]
    fn a_replacement_and_a_reset_reach_the_native_result() {
        let mut event = message();
        event.apply(we::EventOutcome::PluginMessage(
            we::PluginMessageResult::Replace(b"new".to_vec()),
        ));
        assert_eq!(
            event.result(),
            &PluginMessageResult::Replace(Bytes::from_static(b"new"))
        );
        event.apply(we::EventOutcome::PluginMessage(
            we::PluginMessageResult::Forward,
        ));
        assert_eq!(event.result(), &PluginMessageResult::Forward);
    }

    #[test]
    fn a_named_outcome_can_cancel_answer_or_undo_both() {
        let mut event = NamedEvent::new("chat:relay", "text/plain", "hi");
        event.source_plugin = "relay".into();
        let we::Event::NamedEvent(record) = event.to_wit() else {
            panic!("a named event is sent as named-event");
        };
        assert_eq!(record.source_plugin, "relay");
        assert_eq!(record.payload, b"hi");
        assert!(!record.result.cancelled);

        event.apply(we::EventOutcome::NamedEvent(we::NamedEventResult {
            cancelled: true,
            response: Some(we::NamedEventResponse {
                content_type: "text/plain".into(),
                payload: b"pong".to_vec(),
            }),
        }));
        assert!(event.cancelled);
        assert_eq!(
            event.response,
            Some(NamedEventResponse::new("text/plain", "pong"))
        );
        event.apply(we::EventOutcome::NamedEvent(we::NamedEventResult {
            cancelled: false,
            response: None,
        }));
        assert!(!event.cancelled);
        assert_eq!(event.response, None);
    }
}
