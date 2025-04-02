use serde::{Deserialize, Serialize};

use super::{BanAuditLogEntry, BanEntry};

pub mod adapter;
pub mod ban_storage;

/// Ban file storage format
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BanFileStorage {
    bans: Vec<BanEntry>,
    audit_logs: Option<Vec<BanAuditLogEntry>>,
    format_version: u8,
}

/// Audit log file storage format
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuditLogFileStorage {
    audit_logs: Vec<BanAuditLogEntry>,
    format_version: u8,
}

impl Default for BanFileStorage {
    fn default() -> Self {
        Self {
            bans: Vec::new(),
            audit_logs: None,
            format_version: 1,
        }
    }
}

impl Default for AuditLogFileStorage {
    fn default() -> Self {
        Self {
            audit_logs: Vec::new(),
            format_version: 1,
        }
    }
}
