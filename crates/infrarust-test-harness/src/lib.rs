pub mod backend;
pub mod client;
pub mod error;
pub mod framing;
pub mod text;
pub mod versions;
pub mod wire;

use std::time::Duration;

pub use backend::{BackendConn, FakeBackend, FakeBackendBuilder, LoginBehavior, ObservedHandshake};
pub use client::{ClientSession, FakeClient, LoginOutcome, StatusResult};
pub use error::{HarnessError, HarnessResult};
pub use framing::{FrameReader, FrameWriter, FramedConn};
pub use infrarust_protocol::io::PacketFrame;
pub use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
pub use text::DisconnectInfo;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
