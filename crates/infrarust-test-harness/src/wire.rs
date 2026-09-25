use std::sync::OnceLock;

use bytes::Bytes;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::registry::{PacketRegistry, build_default_registry};
use infrarust_protocol::version::ProtocolVersion;

use crate::error::{HarnessError, HarnessResult};

pub fn registry() -> &'static PacketRegistry {
    static REGISTRY: OnceLock<PacketRegistry> = OnceLock::new();
    REGISTRY.get_or_init(build_default_registry)
}

pub fn packet_id<P: Packet>(version: ProtocolVersion) -> HarnessResult<i32> {
    registry().get_packet_id::<P>(version).ok_or_else(|| {
        HarnessError::Unsupported(format!(
            "{} has no {}/{} id in protocol {}",
            P::NAME,
            P::STATE,
            P::DIRECTION,
            version.0
        ))
    })
}

pub fn is<P: Packet>(frame: &PacketFrame, version: ProtocolVersion) -> bool {
    registry().get_packet_id::<P>(version) == Some(frame.id)
}

pub fn encode<P: Packet>(packet: &P, version: ProtocolVersion) -> HarnessResult<PacketFrame> {
    let id = packet_id::<P>(version)?;
    let mut payload = Vec::new();
    packet.encode(&mut payload, version)?;
    Ok(PacketFrame::new(id, Bytes::from(payload)))
}

pub fn decode<P: Packet>(frame: &PacketFrame, version: ProtocolVersion) -> HarnessResult<P> {
    let id = packet_id::<P>(version)?;
    if frame.id != id {
        return Err(HarnessError::Unexpected(format!(
            "expected {} (0x{id:02X}) in protocol {}, got packet 0x{:02X}",
            P::NAME,
            version.0,
            frame.id
        )));
    }
    let mut payload = frame.payload.as_ref();
    Ok(P::decode(&mut payload, version)?)
}
