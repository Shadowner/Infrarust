use bytes::Bytes;

/// An unparsed packet — just a `packet_id` and raw bytes.
///
/// This is the default type for all packets that the proxy doesn't know
/// or doesn't need to inspect. The payload is forwarded as-is to the
/// destination, without any parsing or copy (Bytes is reference-counted).
///
/// In practice, ~95% of Play packets go through this type.
#[derive(Debug, Clone)]
pub struct OpaquePacket {
    /// The packet ID.
    pub id: i32,
    /// The raw payload (after the `packet_id` in the frame).
    pub payload: Bytes,
}
