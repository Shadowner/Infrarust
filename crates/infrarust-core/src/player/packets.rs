//! Packet building helpers for the player command system.
//!
//! Converts API types (`Component`, `TitleData`) into `PacketFrame` values
//! ready to be written to the client bridge.

use bytes::Bytes;

use infrarust_api::types::{Component, TitleData};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::play::chat::{CChatMessageLegacy, CSystemChatMessage};
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::packets::play::title::{
    CSetSubtitle, CSetTitle, CSetTitleTimes, CTitleLegacy,
};
use infrarust_protocol::packets::Packet;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::error::CoreError;

/// Builds a system chat message packet frame.
///
/// Pre-1.19: legacy chat packet with position=1 (system message).
/// 1.19+: CSystemChatMessage (JSON for pre-1.20.3, NBT for 1.20.3+).
pub fn build_system_chat_message(
    component: &Component,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<PacketFrame, CoreError> {
    if version.less_than(ProtocolVersion::V1_19) {
        let packet = CChatMessageLegacy {
            content: component.to_json(),
            position: 1, // system message
        };
        return encode_packet(&packet, version, registry);
    }
    let packet = if version.less_than(ProtocolVersion::V1_20_3) {
        CSystemChatMessage::from_json(&component.to_json(), false)
    } else {
        CSystemChatMessage::from_nbt(component.to_nbt_network(), false)
    };
    encode_packet(&packet, version, registry)
}

/// Builds an action bar packet frame.
///
/// Pre-1.19: legacy chat packet with position=2 (game info / action bar).
/// 1.19+: CSystemChatMessage with `overlay: true`.
pub fn build_action_bar(
    component: &Component,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<PacketFrame, CoreError> {
    if version.less_than(ProtocolVersion::V1_19) {
        let position = if version.no_less_than(ProtocolVersion::V1_8) {
            2
        } else {
            1
        };
        let packet = CChatMessageLegacy {
            content: component.to_json(),
            position,
        };
        return encode_packet(&packet, version, registry);
    }
    let packet = if version.less_than(ProtocolVersion::V1_20_3) {
        CSystemChatMessage::from_json(&component.to_json(), true)
    } else {
        CSystemChatMessage::from_nbt(component.to_nbt_network(), true)
    };
    encode_packet(&packet, version, registry)
}

/// Builds a play-state disconnect packet frame.
///
/// Encodes the reason as JSON for pre-1.20.3 or Network NBT for 1.20.3+.
pub fn build_disconnect(
    reason: &Component,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<PacketFrame, CoreError> {
    let packet = if version.less_than(ProtocolVersion::V1_20_3) {
        CDisconnect::from_json(&reason.to_json())
    } else {
        CDisconnect::from_nbt(reason.to_nbt_network())
    };
    encode_packet(&packet, version, registry)
}

/// Builds the title packets (title text, subtitle text, timing).
///
/// Pre-1.8: no title support, returns empty vec.
/// 1.8–1.16: legacy combined title packet with action discriminator.
/// 1.17+: separate CSetTitle/CSetSubtitle/CSetTitleTimes packets.
pub fn build_title_packets(
    title: &TitleData,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<Vec<PacketFrame>, CoreError> {
    if version.less_than(ProtocolVersion::V1_8) {
        return Ok(Vec::new());
    }

    if version.less_than(ProtocolVersion::V1_17) {
        let mut frames = Vec::with_capacity(3);
        frames.push(encode_packet(
            &CTitleLegacy::SetTimes {
                fade_in: title.fade_in_ticks,
                stay: title.stay_ticks,
                fade_out: title.fade_out_ticks,
            },
            version,
            registry,
        )?);
        frames.push(encode_packet(
            &CTitleLegacy::SetSubtitle(title.subtitle.to_json()),
            version,
            registry,
        )?);
        frames.push(encode_packet(
            &CTitleLegacy::SetTitle(title.title.to_json()),
            version,
            registry,
        )?);
        return Ok(frames);
    }

    let mut frames = Vec::with_capacity(3);

    // 1. Title times (sent first so they apply before the title shows)
    let times = CSetTitleTimes {
        fade_in: title.fade_in_ticks,
        stay: title.stay_ticks,
        fade_out: title.fade_out_ticks,
    };
    frames.push(encode_packet(&times, version, registry)?);

    // 2. Subtitle (sent before title so it's visible when title appears)
    let subtitle = if version.less_than(ProtocolVersion::V1_20_3) {
        CSetSubtitle::from_json(&title.subtitle.to_json())
    } else {
        CSetSubtitle::from_nbt(title.subtitle.to_nbt_network())
    };
    frames.push(encode_packet(&subtitle, version, registry)?);

    // 3. Title text (triggers the display)
    let title_pkt = if version.less_than(ProtocolVersion::V1_20_3) {
        CSetTitle::from_json(&title.title.to_json())
    } else {
        CSetTitle::from_nbt(title.title.to_nbt_network())
    };
    frames.push(encode_packet(&title_pkt, version, registry)?);

    Ok(frames)
}

/// Encodes a typed packet into a `PacketFrame`.
pub(crate) fn encode_packet<P: Packet + 'static>(
    packet: &P,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<PacketFrame, CoreError> {
    let packet_id = registry
        .get_packet_id::<P>(ConnectionState::Play, Direction::Clientbound, version)
        .ok_or_else(|| {
            CoreError::Other(format!(
                "no packet ID for {} in Play/Clientbound/{version:?}",
                P::NAME,
            ))
        })?;

    let mut payload = Vec::new();
    packet
        .encode(&mut payload, version)
        .map_err(|e| CoreError::Other(e.to_string()))?;

    Ok(PacketFrame {
        id: packet_id,
        payload: Bytes::from(payload),
    })
}
