//! Packet building helpers for the player command system.
//!
//! Converts API types (`Component`, `TitleData`) into `PacketFrame` values
//! ready to be written to the client bridge.

use bytes::Bytes;
use uuid::Uuid;

use infrarust_api::player::{BossBar, BossBarUpdate, clamp_progress};
use infrarust_api::types::{Component, TitleData};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::play::boss_bar::{BossBarAction, CBossBar};
use infrarust_protocol::packets::play::chat::{CChatMessageLegacy, CSystemChatMessage};
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::packets::play::tab_list::CTabListHeaderFooter;
use infrarust_protocol::packets::play::title::{
    CClearTitles, CSetSubtitle, CSetTitle, CSetTitleTimes, CTitleLegacy,
};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::util::text::{encode_text_component, json_for, nbt_for};

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
            content: json_for(component, version),
            position: 1, // system message
        };
        return encode_packet(&packet, version, registry);
    }
    let packet = if version.less_than(ProtocolVersion::V1_20_3) {
        CSystemChatMessage::from_json(&json_for(component, version), false)
    } else {
        CSystemChatMessage::from_nbt(nbt_for(component, version), false)
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
            content: json_for(component, version),
            position,
        };
        return encode_packet(&packet, version, registry);
    }
    let packet = if version.less_than(ProtocolVersion::V1_20_3) {
        CSystemChatMessage::from_json(&json_for(component, version), true)
    } else {
        CSystemChatMessage::from_nbt(nbt_for(component, version), true)
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
    let packet = CDisconnect {
        reason: encode_text_component(reason, version, ConnectionState::Play),
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
        return Ok(vec![
            encode_packet(
                &CTitleLegacy::SetTimes {
                    fade_in: title.fade_in_ticks,
                    stay: title.stay_ticks,
                    fade_out: title.fade_out_ticks,
                },
                version,
                registry,
            )?,
            encode_packet(
                &CTitleLegacy::SetSubtitle(json_for(&title.subtitle, version)),
                version,
                registry,
            )?,
            encode_packet(
                &CTitleLegacy::SetTitle(json_for(&title.title, version)),
                version,
                registry,
            )?,
        ]);
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
        CSetSubtitle::from_json(&json_for(&title.subtitle, version))
    } else {
        CSetSubtitle::from_nbt(nbt_for(&title.subtitle, version))
    };
    frames.push(encode_packet(&subtitle, version, registry)?);

    // 3. Title text (triggers the display)
    let title_pkt = if version.less_than(ProtocolVersion::V1_20_3) {
        CSetTitle::from_json(&json_for(&title.title, version))
    } else {
        CSetTitle::from_nbt(nbt_for(&title.title, version))
    };
    frames.push(encode_packet(&title_pkt, version, registry)?);

    Ok(frames)
}

pub fn build_header_footer(
    header: &Component,
    footer: &Component,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<PacketFrame, CoreError> {
    let packet = CTabListHeaderFooter {
        header: encode_text_component(header, version, ConnectionState::Play),
        footer: encode_text_component(footer, version, ConnectionState::Play),
    };
    encode_packet(&packet, version, registry)
}

pub fn build_clear_title(
    reset: bool,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<Option<PacketFrame>, CoreError> {
    if version.less_than(ProtocolVersion::V1_8) {
        return Ok(None);
    }
    if version.less_than(ProtocolVersion::V1_17) {
        let packet = if reset {
            CTitleLegacy::Reset
        } else {
            CTitleLegacy::Hide
        };
        return encode_packet(&packet, version, registry).map(Some);
    }
    encode_packet(&CClearTitles { reset }, version, registry).map(Some)
}

pub(crate) fn boss_bar_added(id: Uuid, bar: &BossBar, version: ProtocolVersion) -> CBossBar {
    CBossBar {
        id,
        action: BossBarAction::Add {
            title: encode_text_component(&bar.title, version, ConnectionState::Play),
            health: clamp_progress(bar.progress),
            color: bar.color.id(),
            division: bar.overlay.id(),
            flags: bar.flags.bits(),
        },
    }
}

pub(crate) fn boss_bar_updated(
    id: Uuid,
    update: &BossBarUpdate,
    version: ProtocolVersion,
) -> Option<CBossBar> {
    let action = match update {
        BossBarUpdate::Title(title) => {
            BossBarAction::UpdateTitle(encode_text_component(title, version, ConnectionState::Play))
        }
        BossBarUpdate::Progress(progress) => BossBarAction::UpdateHealth(clamp_progress(*progress)),
        BossBarUpdate::Style { color, overlay } => BossBarAction::UpdateStyle {
            color: color.id(),
            division: overlay.id(),
        },
        BossBarUpdate::Flags(flags) => BossBarAction::UpdateFlags(flags.bits()),
        _ => return None,
    };
    Some(CBossBar { id, action })
}

pub(crate) const fn boss_bar_removed(id: Uuid) -> CBossBar {
    CBossBar {
        id,
        action: BossBarAction::Remove,
    }
}

/// Encodes a typed packet into a `PacketFrame`.
pub(crate) fn encode_packet<P: Packet>(
    packet: &P,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<PacketFrame, CoreError> {
    let packet_id = registry.get_packet_id::<P>(version).ok_or_else(|| {
        CoreError::Other(format!(
            "no packet ID for {} in {}/{}/{version:?}",
            P::NAME,
            P::STATE,
            P::DIRECTION,
        ))
    })?;

    let mut payload = Vec::new();
    packet
        .encode(&mut payload, version)
        .map_err(|e| CoreError::Other(e.to_string()))?;

    Ok(PacketFrame::new(packet_id, Bytes::from(payload)))
}
