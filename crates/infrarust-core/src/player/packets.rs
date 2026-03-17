//! Packet building helpers for the player command system.
//!
//! Converts API types (`Component`, `TitleData`) into `PacketFrame` values
//! ready to be written to the client bridge.

use bytes::Bytes;

use infrarust_api::types::{Component, TitleData};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::play::chat::CSystemChatMessage;
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::packets::play::title::{CSetSubtitle, CSetTitle, CSetTitleTimes};
use infrarust_protocol::packets::Packet;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::error::CoreError;

/// Builds a system chat message packet frame.
pub fn build_system_chat_message(
    component: &Component,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<PacketFrame, CoreError> {
    let packet = CSystemChatMessage::from_json(&component.to_json(), false);
    encode_packet(&packet, version, registry)
}

/// Builds an action bar packet frame (system chat with `overlay: true`).
pub fn build_action_bar(
    component: &Component,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<PacketFrame, CoreError> {
    let packet = CSystemChatMessage::from_json(&component.to_json(), true);
    encode_packet(&packet, version, registry)
}

/// Builds a play-state disconnect packet frame.
pub fn build_disconnect(
    reason: &Component,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<PacketFrame, CoreError> {
    let packet = CDisconnect::from_json(&reason.to_json());
    encode_packet(&packet, version, registry)
}

/// Builds the three title packets (title text, subtitle text, timing).
pub fn build_title_packets(
    title: &TitleData,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<Vec<PacketFrame>, CoreError> {
    let mut frames = Vec::with_capacity(3);

    // 1. Title times (sent first so they apply before the title shows)
    let times = CSetTitleTimes {
        fade_in: title.fade_in_ticks,
        stay: title.stay_ticks,
        fade_out: title.fade_out_ticks,
    };
    frames.push(encode_packet(&times, version, registry)?);

    // 2. Subtitle (sent before title so it's visible when title appears)
    let subtitle = CSetSubtitle::from_json(&title.subtitle.to_json());
    frames.push(encode_packet(&subtitle, version, registry)?);

    // 3. Title text (triggers the display)
    let title_pkt = CSetTitle::from_json(&title.title.to_json());
    frames.push(encode_packet(&title_pkt, version, registry)?);

    Ok(frames)
}

/// Encodes a typed packet into a `PacketFrame`.
fn encode_packet<P: Packet + 'static>(
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
