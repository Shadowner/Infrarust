use bytes::Bytes;

#[derive(Debug, Clone)]
pub struct PacketFrame {
    pub id: i32,
    pub payload: Bytes,
    raw: Option<RawWire>,
}

#[derive(Debug, Clone)]
struct RawWire {
    bytes: Bytes,
    threshold: Option<i32>,
}

impl PacketFrame {
    #[must_use]
    pub const fn new(id: i32, payload: Bytes) -> Self {
        Self {
            id,
            payload,
            raw: None,
        }
    }

    pub(crate) fn with_raw(id: i32, payload: Bytes, bytes: Bytes, threshold: Option<i32>) -> Self {
        Self {
            id,
            payload,
            raw: Some(RawWire { bytes, threshold }),
        }
    }

    #[must_use]
    pub fn raw_wire(&self, threshold: Option<i32>) -> Option<&[u8]> {
        self.raw
            .as_ref()
            .filter(|raw| raw.threshold == threshold)
            .map(|raw| raw.bytes.as_ref())
    }

    pub fn strip_raw(&mut self) {
        self.raw = None;
    }
}
