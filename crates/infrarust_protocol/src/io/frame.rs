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
    format: WireFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireFormat {
    Plain,
    Compressed {
        deflated: bool,
        uncompressed_len: usize,
    },
}

impl WireFormat {
    #[must_use]
    pub(crate) const fn conforms_to(self, threshold: Option<i32>) -> bool {
        match (self, threshold) {
            (Self::Plain, None) => true,
            (
                Self::Compressed {
                    deflated,
                    uncompressed_len,
                },
                Some(threshold),
            ) => deflated == should_compress(uncompressed_len, threshold),
            (Self::Plain, Some(_)) | (Self::Compressed { .. }, None) => false,
        }
    }
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

    pub(crate) const fn with_raw(id: i32, payload: Bytes, bytes: Bytes, format: WireFormat) -> Self {
        Self {
            id,
            payload,
            raw: Some(RawWire { bytes, format }),
        }
    }

    #[must_use]
    pub fn raw_wire(&self, threshold: Option<i32>) -> Option<&[u8]> {
        self.raw
            .as_ref()
            .filter(|raw| raw.format.conforms_to(threshold))
            .map(|raw| raw.bytes.as_ref())
    }
}

#[must_use]
pub(crate) const fn should_compress(uncompressed_len: usize, threshold: i32) -> bool {
    threshold >= 0 && uncompressed_len >= threshold as usize
}
