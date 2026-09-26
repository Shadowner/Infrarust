use infrarust_api::event::ConnectionState;
use infrarust_api::filter::{
    CodecFilterError, CodecSessionInit, CodecVerdict, ConnectionSide, FrameOutput,
};
use infrarust_api::types::RawPacket;

use super::bindings::exports::infrarust::plugin::codec_filter as wit_codec;
use crate::bindings::infrarust::plugin::types as wit_types;
use crate::convert::{ip_to_wit, raw_packet_from_wit, socket_to_wit, wit_enum_map};

pub(crate) fn session_init_to_wit(init: &CodecSessionInit) -> wit_codec::CodecSessionInit {
    wit_codec::CodecSessionInit {
        client_version: init.client_version.raw(),
        connection_id: init.connection_id,
        remote_addr: socket_to_wit(init.remote_addr),
        real_ip: init.real_ip.map(ip_to_wit),
        side: connection_side_to_wit(init.side),
    }
}

wit_enum_map!(connection_state_to_wit: ConnectionState => wit_types::ConnectionState {
    Handshake, Status, Login, Configuration, Play
} else Play);

wit_enum_map!(connection_side_to_wit: ConnectionSide => wit_codec::ConnectionSide {
    ClientSide, ServerSide
});

pub(crate) fn apply_filter_output(
    out: wit_codec::FilterOutput,
    packet: &mut RawPacket,
    output: &mut FrameOutput,
) -> CodecVerdict {
    match out {
        wit_codec::FilterOutput::Pass => CodecVerdict::Pass,
        wit_codec::FilterOutput::Drop => CodecVerdict::Drop,
        wit_codec::FilterOutput::PassModified(extras) => {
            apply_extras(extras, packet, output);
            CodecVerdict::Pass
        }
        wit_codec::FilterOutput::Replace(extras) => {
            apply_extras(extras, packet, output);
            CodecVerdict::Replace
        }
        wit_codec::FilterOutput::Error(e) => CodecVerdict::Error(codec_error_from_wit(e)),
    }
}

fn apply_extras(extras: wit_codec::FilterExtras, packet: &mut RawPacket, output: &mut FrameOutput) {
    if let Some(p) = extras.packet {
        *packet = raw_packet_from_wit(p);
    }
    push_injections(extras.inject_before, extras.inject_after, output);
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
        wit_codec::CodecFilterError::UnsupportedVersion(v) => {
            CodecFilterError::UnsupportedVersion(v)
        }
        wit_codec::CodecFilterError::Internal(s) => CodecFilterError::Internal(s),
    }
}
