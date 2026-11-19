use bytes::{Bytes, BytesMut};

use infrarust_protocol::ProtocolError;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::play::dimension::DimensionInfo;
use infrarust_protocol::packets::play::join_game::CJoinGame;
use infrarust_protocol::packets::play::respawn_switch;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;

pub(crate) fn packet_id<P: Packet>(
    registry: &PacketRegistry,
    version: ProtocolVersion,
) -> Result<i32, CoreError> {
    registry.get_packet_id::<P>(version).ok_or_else(|| {
        CoreError::Protocol(ProtocolError::invalid(format!(
            "no packet id for {} ({} {}) at protocol {version}",
            P::NAME,
            P::STATE,
            P::DIRECTION,
        )))
    })
}

pub(crate) fn respawn_frame(
    dimension: &DimensionInfo,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<PacketFrame, CoreError> {
    let respawn = respawn_switch::for_switch(dimension, version)?;
    encode_packet(&respawn, version, registry)
}

pub(crate) fn announce_online_mode(
    frame: &mut PacketFrame,
    online_mode: bool,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) {
    if registry.get_packet_id::<CJoinGame>(version) != Some(frame.id) {
        return;
    }
    let flag = u8::from(online_mode);
    match CJoinGame::online_mode_offset(&frame.payload, version) {
        Ok(Some(at)) if frame.payload[at] != flag => {
            let mut payload = BytesMut::from(frame.payload.as_ref());
            payload[at] = flag;
            *frame = PacketFrame::new(frame.id, payload.freeze());
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!("relaying the backend's join game with its own online mode: {e}");
        }
    }
}

pub(crate) fn encode_packet<P: Packet>(
    packet: &P,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<PacketFrame, CoreError> {
    let packet_id = packet_id::<P>(registry, version)?;
    let mut payload = Vec::new();
    packet.encode(&mut payload, version)?;
    Ok(PacketFrame::new(packet_id, Bytes::from(payload)))
}

pub(crate) fn encode_for_state<P: Packet>(
    packet: &P,
    state: ConnectionState,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<PacketFrame, CoreError> {
    if state != P::STATE {
        return Err(CoreError::Protocol(ProtocolError::invalid(format!(
            "cannot send {} ({}) while the bridge is in {state}",
            P::NAME,
            P::STATE,
        ))));
    }
    encode_packet(packet, version, registry)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use infrarust_protocol::io::{PacketDecoder, PacketEncoder};

    use super::*;
    use crate::limbo::spawn::build_limbo_join_game;

    const VERSION: ProtocolVersion = ProtocolVersion::V26_3;
    const THRESHOLDS: [Option<i32>; 3] = [None, Some(256), Some(0)];

    fn registry() -> PacketRegistry {
        infrarust_protocol::build_default_registry()
    }

    fn relayed(frame: &PacketFrame, threshold: Option<i32>) -> PacketFrame {
        let mut encoder = PacketEncoder::new();
        let mut decoder = PacketDecoder::new();
        if let Some(threshold) = threshold {
            encoder.set_compression(threshold);
            decoder.set_compression(threshold);
        }
        encoder.append_frame(frame).unwrap();
        decoder.queue_bytes(&encoder.take());
        decoder.try_next_frame().unwrap().unwrap()
    }

    fn join_game(online_mode: bool, version: ProtocolVersion) -> PacketFrame {
        let join = build_limbo_join_game(version, online_mode).unwrap();
        encode_packet(&join, version, &registry()).unwrap()
    }

    #[test]
    fn the_sessions_online_mode_replaces_the_backends_on_the_wire() {
        let registry = registry();
        for threshold in THRESHOLDS {
            for (backend, session) in [(false, true), (true, false), (true, true), (false, false)] {
                let mut frame = relayed(&join_game(backend, VERSION), threshold);
                announce_online_mode(&mut frame, session, VERSION, &registry);
                assert_eq!(
                    relayed(&frame, threshold).payload,
                    join_game(session, VERSION).payload,
                    "{backend} {session} {threshold:?}"
                );
            }
        }
    }

    #[test]
    fn a_frame_without_the_flag_is_left_alone() {
        let registry = registry();
        let old = ProtocolVersion::V26_1;
        let before_the_flag = join_game(false, old);
        let mut frame = before_the_flag.clone();
        announce_online_mode(&mut frame, true, old, &registry);
        assert_eq!(frame.payload, before_the_flag.payload);

        let join = join_game(false, VERSION);
        let mut other = PacketFrame::new(join.id + 1, join.payload.clone());
        announce_online_mode(&mut other, true, VERSION, &registry);
        assert_eq!(other.payload, join.payload);

        let mut truncated = PacketFrame::new(join.id, Bytes::from_static(&[0, 0, 0, 1, 0]));
        announce_online_mode(&mut truncated, true, VERSION, &registry);
        assert_eq!(truncated.payload.as_ref(), [0, 0, 0, 1, 0]);
    }
}
