use infrarust_api::services::{AvailableUpdate, InstallState, RestartRequirement, UpdateStatus};
use serde::Serialize;

use crate::util::format_system_time;

#[derive(Debug, Serialize)]
pub struct UpdateStatusResponse {
    pub enabled: bool,
    pub current_version: String,
    pub checks_enabled: bool,
    pub last_check: Option<String>,
    pub last_error: Option<String>,
    pub available: Option<AvailableUpdateResponse>,
    pub install: InstallStateResponse,
    pub install_refusal: Option<String>,
    pub binary_changed: bool,
}

impl UpdateStatusResponse {
    pub fn from_status(status: UpdateStatus) -> Self {
        Self {
            enabled: true,
            current_version: status.current_version,
            checks_enabled: status.checks_enabled,
            last_check: status.last_check.map(format_system_time),
            last_error: status.last_error,
            available: status.available.map(AvailableUpdateResponse::from),
            install: status.install.into(),
            install_refusal: status.install_refusal,
            binary_changed: status.binary_changed,
        }
    }

    pub fn disabled(current_version: String) -> Self {
        Self {
            enabled: false,
            current_version,
            checks_enabled: false,
            last_check: None,
            last_error: None,
            available: None,
            install: InstallStateResponse::Idle,
            install_refusal: None,
            binary_changed: false,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AvailableUpdateResponse {
    pub version: String,
    pub published_at: Option<String>,
    pub notes_url: Option<String>,
    pub security: bool,
    pub restart: &'static str,
}

impl From<AvailableUpdate> for AvailableUpdateResponse {
    fn from(update: AvailableUpdate) -> Self {
        Self {
            version: update.version,
            published_at: update.published_at,
            notes_url: update.notes_url,
            security: update.security,
            restart: restart_name(update.restart),
        }
    }
}

const fn restart_name(restart: RestartRequirement) -> &'static str {
    match restart {
        RestartRequirement::Hot => "hot",
        RestartRequirement::Full => "full",
        RestartRequirement::Manual => "manual",
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InstallStateResponse {
    Idle,
    Running { version: String },
    Installed { version: String },
    Failed { reason: String },
}

impl From<InstallState> for InstallStateResponse {
    fn from(state: InstallState) -> Self {
        match state {
            InstallState::Idle => Self::Idle,
            InstallState::Running { version } => Self::Running { version },
            InstallState::Installed { version } => Self::Installed { version },
            InstallState::Failed { reason } => Self::Failed { reason },
        }
    }
}
