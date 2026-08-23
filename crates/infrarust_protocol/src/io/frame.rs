use bytes::Bytes;

/// A packet after framing (length and `packet_id` decoded)
/// but BEFORE payload parsing.
///
/// This is the central type of the proxy. Every packet received from the network
/// passes through here. The registry then decides: parse into a typed struct,
/// or forward as opaque.
///
/// The `payload` uses `bytes::Bytes` (reference-counted, zero-copy clone).
///
/// Frames decoded from the wire also remember their original framed bytes
/// (see [`raw_wire`](Self::raw_wire)), which lets an encoder forward them
/// verbatim — skipping re-compression — as long as the frame was not modified.
/// The field is private on purpose: any frame built or rebuilt through
/// [`new`](Self::new) carries no wire bytes and gets re-encoded, so a stale
/// copy can never be emitted for a modified packet.
#[derive(Debug, Clone)]
pub struct PacketFrame {
    /// The packet ID (already decoded from the `VarInt` in the frame).
    pub id: i32,
    /// The raw payload after the `packet_id`.
    /// Uses `Bytes` for zero-copy: `.clone()` is an Arc increment.
    pub payload: Bytes,
    raw: Option<RawWire>,
}

/// The exact bytes a frame arrived as (length prefix included), plus the
/// compression threshold the decoder was using when it captured them.
#[derive(Debug, Clone)]
struct RawWire {
    bytes: Bytes,
    threshold: Option<i32>,
}

impl PacketFrame {
    /// Creates a frame from its parts. The frame carries no original wire
    /// bytes, so encoders will (re-)encode it.
    #[must_use]
    pub const fn new(id: i32, payload: Bytes) -> Self {
        Self {
            id,
            payload,
            raw: None,
        }
    }

    /// Creates a frame that remembers the wire bytes it was decoded from.
    pub(crate) fn with_raw(id: i32, payload: Bytes, bytes: Bytes, threshold: Option<i32>) -> Self {
        Self {
            id,
            payload,
            raw: Some(RawWire { bytes, threshold }),
        }
    }

    /// Returns the original wire bytes if they are valid verbatim for a
    /// destination using `threshold` — i.e. they were captured under the
    /// exact same compression threshold the destination encoder would frame
    /// them with. Returns `None` for frames built via [`new`](Self::new).
    #[must_use]
    pub fn raw_wire(&self, threshold: Option<i32>) -> Option<&[u8]> {
        self.raw
            .as_ref()
            .filter(|raw| raw.threshold == threshold)
            .map(|raw| raw.bytes.as_ref())
    }

    /// Drops the remembered wire bytes, forcing re-encoding. Use before
    /// storing a frame beyond its session (the wire bytes pin the read
    /// buffer they were captured from).
    pub fn strip_raw(&mut self) {
        self.raw = None;
    }
}

#[must_use]
pub(crate) const fn should_compress(uncompressed_len: usize, threshold: i32) -> bool {
    threshold >= 0 && uncompressed_len >= threshold as usize
}
