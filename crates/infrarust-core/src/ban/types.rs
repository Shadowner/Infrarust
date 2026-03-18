//! Ban system data types.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub use infrarust_api::services::ban_service::{BanEntry, BanTarget};
#[allow(unused_imports)] // used by serde(with) on BanAuditLogEntry
use infrarust_api::services::ban_service::epoch_serde;

// ─────────────────────────── Audit Log ───────────────────────────

/// Type of action in the audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BanAction {
    /// A ban was added.
    Ban,
    /// A ban was manually lifted.
    Unban,
    /// A ban expired and was purged.
    Expired,
}

/// Audit log entry tracking a ban/unban operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanAuditLogEntry {
    /// Type of action.
    pub action: BanAction,
    /// Target of the action.
    pub target: BanTarget,
    /// Reason (for bans).
    pub reason: Option<String>,
    /// Source of the action.
    pub source: String,
    /// Timestamp of the action.
    #[serde(with = "epoch_serde")]
    pub timestamp: SystemTime,
}
