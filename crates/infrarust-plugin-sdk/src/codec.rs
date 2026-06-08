//! Ergonomic guest-side codec-filter authoring.
//!
//! ```ignore
//! struct Flip;
//! impl CodecFilter for Flip {
//!     fn filter(&mut self, _c: &CodecContext, p: &mut Packet, _o: &mut Injections) -> Verdict {
//!         if let Some(b) = p.data_mut().first_mut() { *b ^= 0xff; }
//!         Verdict::Pass
//!     }
//! }
//!
//! #[plugin]
//! impl Plugin for MyPlugin {
//!     fn register_codec_filters(reg: &mut CodecRegistrar) {
//!         reg.add("flip", FilterPriority::Normal, |_init| Box::new(Flip));
//!     }
//! }
//! ```
//!
//! The constructor is an associated-fn context (no plugin `self`): codec filters
//! are per-connection and stateless across connections, so each one builds its own
//! state from the [`CodecSessionInit`].

pub use crate::bindings::codec_filter::{
    CodecContext, CodecSessionInit, ConnectionSide, ConnectionState, PlayerInfo,
};
use crate::bindings::codec_filter::RawPacket;

/// A framed Minecraft packet. Reads are free; any mutation marks the packet dirty
/// so the host only re-copies it when it actually changed (`none` = zero-copy pass).
pub struct Packet {
    id: i32,
    data: Vec<u8>,
    dirty: bool,
}

impl Packet {
    #[must_use]
    pub fn new(id: i32, data: Vec<u8>) -> Self {
        Self {
            id,
            data,
            dirty: true,
        }
    }

    #[must_use]
    pub fn id(&self) -> i32 {
        self.id
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn set_id(&mut self, id: i32) {
        self.id = id;
        self.dirty = true;
    }

    pub fn set_data(&mut self, data: Vec<u8>) {
        self.data = data;
        self.dirty = true;
    }

    pub fn data_mut(&mut self) -> &mut Vec<u8> {
        self.dirty = true;
        &mut self.data
    }

    fn from_wit(p: RawPacket) -> Self {
        Self {
            id: p.packet_id,
            data: p.data,
            dirty: false,
        }
    }

    fn into_wit(self) -> RawPacket {
        RawPacket {
            packet_id: self.id,
            data: self.data,
        }
    }
}

pub enum Verdict {
    Pass,
    Drop,
    Replace,
}

#[derive(Default)]
pub struct Injections {
    before: Vec<Packet>,
    after: Vec<Packet>,
}

impl Injections {
    pub fn before(&mut self, packet: Packet) {
        self.before.push(packet);
    }

    pub fn after(&mut self, packet: Packet) {
        self.after.push(packet);
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum FilterPriority {
    First,
    Early,
    #[default]
    Normal,
    Late,
    Last,
}

impl FilterPriority {
    fn to_u8(self) -> u8 {
        match self {
            FilterPriority::First => 0,
            FilterPriority::Early => 1,
            FilterPriority::Normal => 2,
            FilterPriority::Late => 3,
            FilterPriority::Last => 4,
        }
    }
}

pub trait CodecFilter {
    /// Inspect/modify a single packet. Mutating `packet` is applied; push frames
    /// into `out` to inject around it.
    fn filter(&mut self, ctx: &CodecContext, packet: &mut Packet, out: &mut Injections) -> Verdict;

    /// Protocol state changed (e.g. Login → Configuration → Play).
    fn on_state_change(&mut self, _new_state: ConnectionState) {}
    /// Compression was enabled or its threshold changed.
    fn on_compression_change(&mut self, _threshold: i32) {}
    /// Encryption was enabled.
    fn on_encryption_enabled(&mut self) {}
    /// The connection-side is closing.
    fn on_close(&mut self) {}
}

/// Constructor for a per-connection [`CodecFilter`].
pub type FilterConstructor = Box<dyn Fn(&CodecSessionInit) -> Box<dyn CodecFilter>>;

/// Collects a plugin's codec-filter declarations. Passed to
/// [`Plugin::register_codec_filters`](crate::Plugin::register_codec_filters); the
/// macro replays that method on the main instance (to register with the host) and
/// on each fresh codec instance (to rebuild the constructor table).
pub struct CodecRegistrar {
    pub(crate) notify: bool,
}

impl CodecRegistrar {
    /// Registers a codec filter under `id` with the given ordering `priority`.
    pub fn add(
        &mut self,
        id: &str,
        priority: FilterPriority,
        constructor: impl Fn(&CodecSessionInit) -> Box<dyn CodecFilter> + 'static,
    ) {
        let metadata = crate::bindings::codec_registry::CodecFilterMetadata {
            id: id.to_string(),
            priority: priority.to_u8(),
            after: Vec::new(),
            before: Vec::new(),
        };
        crate::runtime::register_codec_factory(self.notify, metadata, Box::new(constructor));
    }
}

/// Builds a guest `filter-output` from a user verdict + the (possibly mutated)
/// packet + injections. A `Pass` with an unmodified packet sends `none`
/// (zero-copy) so the host doesn't re-copy unchanged bytes.
pub(crate) fn build_filter_output(
    verdict: Verdict,
    packet: Packet,
    injections: Injections,
) -> crate::bindings::codec_filter::FilterOutput {
    use crate::bindings::codec_filter::{CodecVerdict, FilterOutput};
    let before = injections.before.into_iter().map(Packet::into_wit).collect();
    let after = injections.after.into_iter().map(Packet::into_wit).collect();
    match verdict {
        Verdict::Pass => FilterOutput {
            verdict: CodecVerdict::Pass,
            packet: packet.dirty.then(|| packet.into_wit()),
            inject_before: before,
            inject_after: after,
        },
        Verdict::Drop => FilterOutput {
            verdict: CodecVerdict::Drop,
            packet: None,
            inject_before: Vec::new(),
            inject_after: Vec::new(),
        },
        Verdict::Replace => FilterOutput {
            verdict: CodecVerdict::Replace,
            packet: None,
            inject_before: before,
            inject_after: after,
        },
    }
}

pub(crate) fn packet_from_wit(p: RawPacket) -> Packet {
    Packet::from_wit(p)
}
