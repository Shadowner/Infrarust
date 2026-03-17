//! Filter system for packet and transport-level interception.
//!
//! Two filter levels are provided:
//!
//! - **Codec filters** ([`CodecFilterFactory`] / [`CodecFilterInstance`]):
//!   Operate on framed Minecraft packets. Synchronous, per-connection instances.
//!   Run on the hot path — must be fast (< 1 us).
//!
//! - **Transport filters** ([`TransportFilter`]):
//!   Operate on raw TCP bytes before Minecraft framing. Async, shared instances.
//!   Can reject connections at the TCP level.

pub mod codec;
pub mod metadata;
pub mod registry;
pub mod transport;

pub use codec::{
    CodecContext, CodecFilterError, CodecFilterFactory, CodecFilterInstance, CodecSessionInit,
    CodecVerdict, ConnectionSide, FrameOutput, PlayerInfo,
};
pub use metadata::{FilterMetadata, FilterPriority};
pub use registry::{CodecFilterRegistry, TransportFilterRegistry};
pub use transport::{FilterVerdict, TransportContext, TransportFilter};
