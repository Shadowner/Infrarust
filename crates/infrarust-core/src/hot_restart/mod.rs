use std::time::{SystemTime, UNIX_EPOCH};

pub mod announce;
pub mod coordinator;
pub mod freeze;
#[cfg(any(test, feature = "test-support"))]
pub mod memory;
pub mod package;
pub mod platform;
pub mod preflight;
pub mod relays;
pub mod restore;
pub mod resume;
pub mod state;

pub use coordinator::{RestartHandle, RestartHub, RestartPhase};
#[cfg(any(test, feature = "test-support"))]
pub use memory::{InheritanceOutcome, InheritanceRecord, MemoryInheritance};
pub use package::{
    DroppedSession, FrozenSession, Handle, HandleTable, HandoverPackage, ListenerHandover,
    UnpackFailure, Unpacked,
};
pub use platform::{Handed, Inheritance, RestartPlan, Successor};
pub use preflight::{BinaryPreflight, Preflight, PreflightError};
pub use relays::{InFlight, LoginLink, RelayRegistry, RelayTicket};
pub use restore::{Inherited, PrepareFailure, Prepared, RestoreError, Restored};
pub use resume::{
    RESUME_COOKIE, Redeemed, Refusal, ResumeHolder, ResumePresenter, ResumeRecord, ResumeRegistry,
    ResumeToken,
};
pub use state::{HandleId, HandoverState, SessionMeta};

#[derive(Debug, thiserror::Error)]
pub enum HandoverError {
    #[error("hot restart is not supported on {0}")]
    Unsupported(&'static str),
    #[error("the handover state is malformed: {0}")]
    Malformed(String),
    #[error("handover protocol {found} is not accepted (accepted: {accepted:?})")]
    Protocol {
        found: u32,
        accepted: &'static [u32],
    },
    #[error("handle {0} is missing")]
    MissingHandle(u32),
    #[error("handle {0} is not a {1}")]
    WrongHandle(u32, &'static str),
    #[error("handle {0} does not match the handover state: {1}")]
    Mismatch(u32, String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("the handover was rolled back: {0}")]
    RolledBack(String),
    #[error("the successor gave the handover back: {reason}")]
    Returned {
        reason: String,
        package: Box<HandoverPackage>,
    },
}

fn unix_millis(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH).map_or(0, |since| {
        u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
    })
}

fn now_ms() -> u64 {
    unix_millis(SystemTime::now())
}
