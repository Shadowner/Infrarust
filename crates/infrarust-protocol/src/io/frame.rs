use bytes::Bytes;

/// A packet after framing (length and packet_id decoded)
/// but BEFORE payload parsing.
///
/// This is the central type of the proxy. Every packet received from the network
/// passes through here. The registry then decides: parse into a typed struct,
/// or forward as opaque.
///
/// The `payload` uses `bytes::Bytes` (reference-counted, zero-copy clone).
#[derive(Debug, Clone)]
pub struct PacketFrame {
    /// The packet ID (already decoded from the VarInt in the frame).
    pub id: i32,
    /// The raw payload after the packet_id.
    /// Uses `Bytes` for zero-copy: `.clone()` is an Arc increment.
    pub payload: Bytes,
}
