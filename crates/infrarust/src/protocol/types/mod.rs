mod angle;
mod arrays;
mod primitives;
mod strings;
mod traits;
mod uuid;
mod var_numbers;

// Re-exports publics
pub use angle::Angle;
pub use arrays::{ByteArray, PrefixedArray};
pub use primitives::{Boolean, Byte, Double, Float, Int, Long, Short, UnsignedShort};
pub use strings::{Chat, Identifier, ProtocolString};
pub use traits::{AsyncProtocolWrite, ProtocolRead, ProtocolWrite, WriteToBytes};
pub use uuid::ProtocolUUID;
pub use var_numbers::{VarInt, VarLong};

// Constants
pub const MAX_VARINT_LEN: usize = 5;
pub const MAX_VARLONG_LEN: usize = 10;
