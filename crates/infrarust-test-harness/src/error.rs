use std::time::Duration;

use infrarust_core::error::CoreError;
use infrarust_protocol::ProtocolError;

use crate::text::DisconnectInfo;

#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("core error: {0}")]
    Core(#[from] CoreError),

    #[error("timed out after {after:?} waiting for {what}")]
    Timeout { what: String, after: Duration },

    #[error("connection closed while waiting for {0}")]
    Closed(String),

    #[error("disconnected in {} state: {:?}", .0.state, .0.text)]
    Disconnected(Box<DisconnectInfo>),

    #[error("unexpected: {0}")]
    Unexpected(String),

    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl HarnessError {
    pub(crate) fn timeout(what: impl Into<String>, after: Duration) -> Self {
        Self::Timeout {
            what: what.into(),
            after,
        }
    }
}

pub type HarnessResult<T> = Result<T, HarnessError>;
