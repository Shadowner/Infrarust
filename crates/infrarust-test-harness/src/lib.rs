pub mod backend;
pub mod chat;
pub mod client;
pub mod client_replies;
pub mod error;
pub mod framing;
pub mod legacy;
pub mod managed;
pub mod plugin_message;
pub mod proxy;
pub mod recorder;
pub mod scripted;
pub mod session;
pub mod text;
pub mod versions;
pub mod wire;

use std::sync::Once;
use std::time::Duration;

pub use backend::{BackendConn, FakeBackend, FakeBackendBuilder, LoginBehavior, ObservedHandshake};
pub use chat::{ChatFrame, ChatPacket};
pub use client::{ClientSession, FakeClient, LoginOutcome, StatusResult};
pub use client_replies::CookieJar;
pub use error::{HarnessError, HarnessResult};
pub use framing::{FrameReader, FrameWriter, FramedConn};
pub use infrarust_protocol::io::PacketFrame;
pub use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
pub use legacy::{
    FakeLegacyBackend, LegacyBackendConn, LegacyClient, LegacyHandshake, LegacyLogin, LegacyPing,
    LegacySession,
};
pub use managed::FakeServerProvider;
pub use proxy::{ServerSpec, TestProxy, TestProxyBuilder};
pub use recorder::{EventKind, RECORDER_PLUGIN_ID, Recorded, Recorder, RecordingPlugin};
pub use scripted::ScriptedPlugin;
pub use session::{FakeSessionServer, SessionCall};
pub use text::DisconnectInfo;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub fn init_tracing() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off"));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_test_writer()
            .try_init();
    });
}
