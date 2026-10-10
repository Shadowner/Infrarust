use std::fmt;

use infrarust_api::events::connection::KickCause;
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::codec::McBufReadExt;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::util::text::{decode_text_component, uses_nbt};

#[derive(Debug, Clone)]
pub struct BackendKick {
    pub frame: PacketFrame,
    pub state: ConnectionState,
    pub reason: Component,
}

impl BackendKick {
    pub fn new(frame: PacketFrame, state: ConnectionState, version: ProtocolVersion) -> Self {
        let reason = decode_reason(&frame, state, version);
        Self {
            frame,
            state,
            reason,
        }
    }

    pub const fn cause(&self) -> KickCause {
        match self.state {
            ConnectionState::Login => KickCause::LoginRefused,
            ConnectionState::Config => KickCause::ConfigDisconnect,
            _ => KickCause::PlayDisconnect,
        }
    }
}

impl fmt::Display for BackendKick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason.to_plain())
    }
}

fn decode_reason(
    frame: &PacketFrame,
    state: ConnectionState,
    version: ProtocolVersion,
) -> Component {
    let payload = frame.payload.as_ref();
    if uses_nbt(version, state) {
        return decode_text_component(payload, version, state);
    }
    let mut reader = payload;
    match reader.read_string() {
        Ok(json) => decode_text_component(json.as_bytes(), version, state),
        Err(_) => decode_text_component(payload, version, state),
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Kick {
    pub(crate) server: ServerId,
    pub(crate) cause: KickCause,
    pub(crate) packet: Option<BackendKick>,
    pub(crate) during_connect: bool,
    pub(crate) stranded: bool,
    message: Option<&'static str>,
}

impl Kick {
    pub(crate) fn from_packet(server: ServerId, packet: BackendKick, during_connect: bool) -> Self {
        Self {
            server,
            cause: packet.cause(),
            packet: Some(packet),
            during_connect,
            stranded: false,
            message: None,
        }
    }

    pub(crate) const fn lost(server: ServerId, during_connect: bool) -> Self {
        Self {
            server,
            cause: KickCause::ConnectionLost,
            packet: None,
            during_connect,
            stranded: false,
            message: None,
        }
    }

    pub(crate) fn unavailable(server: ServerId, message: &'static str, error: String) -> Self {
        Self {
            server,
            cause: KickCause::Unreachable { error },
            packet: None,
            during_connect: true,
            stranded: false,
            message: Some(message),
        }
    }

    pub(crate) fn failed(server: ServerId, error: CoreError, stranded: bool) -> Self {
        let kick = match error {
            CoreError::BackendKick(packet) => Self::from_packet(server, *packet, true),
            error
                if !matches!(error, CoreError::Transport(_)) && error.is_expected_disconnect() =>
            {
                Self::lost(server, true)
            }
            error => Self::unreachable(server, &error),
        };
        Self { stranded, ..kick }
    }

    fn unreachable(server: ServerId, error: &CoreError) -> Self {
        Self {
            server,
            cause: KickCause::Unreachable {
                error: error.to_string(),
            },
            packet: None,
            during_connect: true,
            stranded: false,
            message: None,
        }
    }

    pub(crate) fn reason(&self) -> Option<Component> {
        self.packet
            .as_ref()
            .map(|packet| packet.reason.clone())
            .or_else(|| self.message.map(Component::text))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use bytes::Bytes;
    use infrarust_api::types::NamedColor;
    use infrarust_protocol::codec::McBufWriteExt;

    use super::*;
    use crate::util::text::encode_text_component;

    fn reason() -> Component {
        Component::text("Kicked").color(NamedColor::Red)
    }

    fn string_frame(json: &str) -> PacketFrame {
        let mut payload = Vec::new();
        payload.write_string(json).unwrap();
        PacketFrame::new(0x00, Bytes::from(payload))
    }

    #[test]
    fn reasons_are_parsed_in_every_state() {
        let json = reason().to_json();
        for (state, version) in [
            (ConnectionState::Login, ProtocolVersion::V1_8),
            (ConnectionState::Login, ProtocolVersion::V1_21_11),
            (ConnectionState::Config, ProtocolVersion::V1_20_2),
            (ConnectionState::Play, ProtocolVersion::V1_8),
            (ConnectionState::Play, ProtocolVersion::V1_20_2),
        ] {
            let kick = BackendKick::new(string_frame(&json), state, version);
            assert_eq!(kick.reason, reason(), "{state:?} {version:?}");
        }
        for state in [ConnectionState::Config, ConnectionState::Play] {
            let version = ProtocolVersion::V1_21_11;
            let nbt = encode_text_component(&reason(), version, state);
            let kick = BackendKick::new(PacketFrame::new(0x01, Bytes::from(nbt)), state, version);
            assert_eq!(kick.reason, reason(), "{state:?}");
        }
    }

    #[test]
    fn the_state_names_the_cause() {
        let version = ProtocolVersion::V1_21_11;
        let frame = string_frame("{}");
        assert_eq!(
            BackendKick::new(frame.clone(), ConnectionState::Login, version).cause(),
            KickCause::LoginRefused
        );
        assert_eq!(
            BackendKick::new(frame.clone(), ConnectionState::Config, version).cause(),
            KickCause::ConfigDisconnect
        );
        assert_eq!(
            BackendKick::new(frame, ConnectionState::Play, version).cause(),
            KickCause::PlayDisconnect
        );
    }

    #[test]
    fn failures_are_sorted_by_what_the_backend_did() {
        let server = ServerId::new("lobby");
        let refused = BackendKick::new(
            string_frame(&reason().to_json()),
            ConnectionState::Login,
            ProtocolVersion::V1_21_11,
        );
        let kick = Kick::failed(
            server.clone(),
            CoreError::BackendKick(Box::new(refused)),
            true,
        );
        assert_eq!(kick.cause, KickCause::LoginRefused);
        assert_eq!(kick.reason(), Some(reason()));
        assert!(kick.during_connect && kick.stranded);

        let lost = Kick::failed(server.clone(), CoreError::ConnectionClosed, false);
        assert_eq!(lost.cause, KickCause::ConnectionLost);
        assert_eq!(lost.reason(), None);

        let timeout = Kick::failed(server, CoreError::Timeout("login".into()), false);
        assert_eq!(
            timeout.cause,
            KickCause::Unreachable {
                error: "connection timeout: login".into()
            }
        );
    }
}
