use bytes::Bytes;

use infrarust_protocol::ProtocolError;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::play::dimension::DimensionInfo;
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
