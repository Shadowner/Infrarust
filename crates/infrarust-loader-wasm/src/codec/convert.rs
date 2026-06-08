use infrarust_api::event::ConnectionState;
use infrarust_api::filter::{
    CodecContext, CodecFilterError, CodecSessionInit, CodecVerdict, ConnectionSide, FrameOutput,
    PlayerInfo,
};
use infrarust_api::types::RawPacket;

use super::bindings::exports::infrarust::plugin::codec_filter as wit_codec;
use crate::bindings::infrarust::plugin::types as wit_types;
use crate::convert::raw_packet_from_wit;

pub(crate) fn session_init_to_wit(init: &CodecSessionInit) -> wit_codec::CodecSessionInit {
    wit_codec::CodecSessionInit {
        client_version: init.client_version.raw(),
        connection_id: init.connection_id,
        remote_addr: init.remote_addr.to_string(),
        real_ip: init.real_ip.map(|ip| ip.to_string()),
        side: connection_side_to_wit(init.side),
    }
}

pub(crate) fn codec_context_to_wit(ctx: &CodecContext) -> wit_codec::CodecContext {
    wit_codec::CodecContext {
        client_version: ctx.client_version.raw(),
        server_version: ctx.server_version.map(|v| v.raw()),
        state: connection_state_to_wit(ctx.state),
        connection_id: ctx.connection_id,
        side: connection_side_to_wit(ctx.side),
        player_info: ctx.player_info.as_ref().map(player_info_to_wit),
        is_proxy_consumed: ctx.is_proxy_consumed,
    }
}

pub(crate) fn connection_state_to_wit(state: ConnectionState) -> wit_types::ConnectionState {
    match state {
        ConnectionState::Handshake => wit_types::ConnectionState::Handshake,
        ConnectionState::Status => wit_types::ConnectionState::Status,
        ConnectionState::Login => wit_types::ConnectionState::Login,
        ConnectionState::Configuration => wit_types::ConnectionState::Configuration,
        ConnectionState::Play => wit_types::ConnectionState::Play,
        // `ConnectionState` is #[non_exhaustive]; a future native state has no WIT
        // equivalent yet — wire it explicitly when added.
        _ => wit_types::ConnectionState::Play,
    }
}

fn connection_side_to_wit(side: ConnectionSide) -> wit_codec::ConnectionSide {
    match side {
        ConnectionSide::ClientSide => wit_codec::ConnectionSide::ClientSide,
        ConnectionSide::ServerSide => wit_codec::ConnectionSide::ServerSide,
    }
}

fn player_info_to_wit(info: &PlayerInfo) -> wit_codec::PlayerInfo {
    wit_codec::PlayerInfo {
        username: info.username.clone(),
        uuid: info.uuid.map(|u| u.to_string()),
    }
}

pub(crate) fn apply_filter_output(
    out: wit_codec::FilterOutput,
    packet: &mut RawPacket,
    output: &mut FrameOutput,
) -> CodecVerdict {
    match out.verdict {
        wit_codec::CodecVerdict::Pass => {
            if let Some(p) = out.packet {
                *packet = raw_packet_from_wit(p);
            }
            push_injections(out.inject_before, out.inject_after, output);
            CodecVerdict::Pass
        }
        wit_codec::CodecVerdict::Drop => CodecVerdict::Drop,
        wit_codec::CodecVerdict::Replace => {
            push_injections(out.inject_before, out.inject_after, output);
            CodecVerdict::Replace
        }
        wit_codec::CodecVerdict::Error(e) => CodecVerdict::Error(codec_error_from_wit(e)),
    }
}

fn push_injections(
    before: Vec<wit_types::RawPacket>,
    after: Vec<wit_types::RawPacket>,
    output: &mut FrameOutput,
) {
    for p in before {
        output.inject_before(raw_packet_from_wit(p));
    }
    for p in after {
        output.inject_after(raw_packet_from_wit(p));
    }
}

fn codec_error_from_wit(e: wit_codec::CodecFilterError) -> CodecFilterError {
    match e {
        wit_codec::CodecFilterError::TranslationFailed(s) => CodecFilterError::TranslationFailed(s),
        wit_codec::CodecFilterError::MalformedPayload => CodecFilterError::MalformedPayload,
        wit_codec::CodecFilterError::UnsupportedVersion(v) => CodecFilterError::UnsupportedVersion(v),
        wit_codec::CodecFilterError::Internal(s) => CodecFilterError::Internal(s),
    }
}
