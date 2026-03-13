//! Raw Minecraft packet type.

/// A raw, unparsed Minecraft packet.
///
/// Contains the packet ID and the raw payload bytes. Used in Tier 3
/// virtual backend handlers for full packet-level control.
///
/// # Example
/// ```
/// use infrarust_api::types::RawPacket;
///
/// let packet = RawPacket::new(0x00, bytes::Bytes::from_static(b"\x00"));
/// assert_eq!(packet.packet_id, 0x00);
/// ```
#[derive(Debug, Clone)]
pub struct RawPacket {
    /// The Minecraft packet ID.
    pub packet_id: i32,
    /// The raw packet payload (excluding the packet ID).
    pub data: bytes::Bytes,
}

impl RawPacket {
    /// Creates a new `RawPacket` with the given ID and data.
    #[must_use]
    pub const fn new(packet_id: i32, data: bytes::Bytes) -> Self {
        Self { packet_id, data }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction() {
        let packet = RawPacket::new(0x0E, bytes::Bytes::from_static(b"hello"));
        assert_eq!(packet.packet_id, 0x0E);
        assert_eq!(&packet.data[..], b"hello");
    }

    #[test]
    fn clone() {
        let a = RawPacket::new(1, bytes::Bytes::from_static(b"data"));
        let b = a.clone();
        assert_eq!(a.packet_id, b.packet_id);
        assert_eq!(a.data, b.data);
    }
}
