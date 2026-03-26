//! Convenience re-exports for common protocol types.
//!
//! ```
//! use infrarust_protocol::prelude::*;
//! ```

pub use crate::codec::{Decode, Encode, McBufReadExt, McBufWriteExt, VarInt, VarLong};
pub use crate::error::{ProtocolError, ProtocolResult};
pub use crate::io::{PacketDecoder, PacketEncoder, PacketFrame};
pub use crate::packets::{ErasedPacket, Packet};
pub use crate::registry::{DecodedPacket, PacketRegistry, build_default_registry};
pub use crate::version::{ConnectionState, Direction, ProtocolVersion};
