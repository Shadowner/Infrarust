use std::time::{Duration, SystemTime};

use crate::BoxFuture;

#[doc(hidden)]
pub mod private {
    pub use crate::__sealed::Sealed;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartRequirement {
    Hot,
    Full,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub version: String,
    pub published_at: Option<String>,
    pub notes_url: Option<String>,
    pub security: bool,
    pub restart: RestartRequirement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallState {
    Idle,
    Running { version: String },
    Installed { version: String },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateStatus {
    pub current_version: String,
    pub checks_enabled: bool,
    pub last_check: Option<SystemTime>,
    pub last_error: Option<String>,
    pub available: Option<AvailableUpdate>,
    pub install: InstallState,
    pub install_refusal: Option<String>,
    pub binary_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstallRequest {
    pub version: Option<String>,
    pub then_restart: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateRequestError {
    #[error("an installation is already running")]
    Busy,
    #[error("no update to install")]
    NothingToInstall,
    #[error("{0}")]
    Refused(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RestartRequestError {
    #[error("hot restart is not supported here: {0}")]
    Unsupported(String),
    #[error("a restart is already in progress")]
    InProgress,
    #[error("the proxy is shutting down")]
    ShuttingDown,
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RestartOptions {
    pub delay: Option<Duration>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledRestart {
    pub at: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RestartCancelError {
    #[error("no restart is scheduled")]
    NothingScheduled,
    #[error("the restart is already under way")]
    TooLate,
}

pub trait ProxyUpdates: Send + Sync + crate::__sealed::Sealed {
    fn status(&self) -> UpdateStatus;

    fn check_now(&self) -> BoxFuture<'_, UpdateStatus>;

    fn start_install(&self, request: InstallRequest) -> Result<(), UpdateRequestError>;
}

pub trait ProxyRestart: Send + Sync + crate::__sealed::Sealed {
    fn request_restart(&self) -> Result<(), RestartRequestError> {
        self.request_restart_with(RestartOptions::default())
    }

    fn request_restart_with(&self, options: RestartOptions) -> Result<(), RestartRequestError>;

    fn cancel_restart(&self) -> Result<(), RestartCancelError>;

    fn scheduled_restart(&self) -> Option<ScheduledRestart>;
}
