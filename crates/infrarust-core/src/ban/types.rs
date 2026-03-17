//! Ban system data types.

use std::fmt;
use std::net::IpAddr;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─────────────────────────── Epoch Serde ─────────────────────────

/// Serde helper: serialize `SystemTime` as epoch seconds (u64).
pub mod epoch_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    /// # Errors
    /// Returns the serializer's error type on failure.
    pub fn serialize<S: Serializer>(time: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        let epoch = time
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        epoch.serialize(s)
    }

    /// # Errors
    /// Returns the deserializer's error type on failure.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let epoch = u64::deserialize(d)?;
        Ok(UNIX_EPOCH + Duration::from_secs(epoch))
    }
}

/// Serde helper: serialize `Option<SystemTime>` as optional epoch seconds.
pub mod option_epoch_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    /// # Errors
    /// Returns the serializer's error type on failure.
    pub fn serialize<S: Serializer>(time: &Option<SystemTime>, s: S) -> Result<S::Ok, S::Error> {
        match time {
            Some(t) => {
                let epoch = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                Some(epoch).serialize(s)
            }
            None => Option::<u64>::None.serialize(s),
        }
    }

    /// # Errors
    /// Returns the deserializer's error type on failure.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SystemTime>, D::Error> {
        let opt = Option::<u64>::deserialize(d)?;
        Ok(opt.map(|epoch| UNIX_EPOCH + Duration::from_secs(epoch)))
    }
}

// ─────────────────────────── BanTarget ───────────────────────────

/// Target of a ban. A ban targets exactly one entity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
#[non_exhaustive]
pub enum BanTarget {
    /// Ban by exact IP address (no CIDR).
    Ip(IpAddr),
    /// Ban by Minecraft username (case-insensitive for lookups).
    Username(String),
    /// Ban by Mojang UUID.
    Uuid(Uuid),
}

impl BanTarget {
    /// Returns a human-readable type name for logs and messages.
    pub const fn display_type(&self) -> &'static str {
        match self {
            Self::Ip(_) => "IP",
            Self::Username(_) => "username",
            Self::Uuid(_) => "UUID",
        }
    }
}

impl fmt::Display for BanTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ip(ip) => write!(f, "IP:{ip}"),
            Self::Username(name) => write!(f, "username:{name}"),
            Self::Uuid(uuid) => write!(f, "UUID:{uuid}"),
        }
    }
}

// ─────────────────────────── BanEntry ────────────────────────────

/// An active ban entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanEntry {
    /// The ban target.
    pub target: BanTarget,
    /// Reason for the ban (shown to the player on kick).
    pub reason: Option<String>,
    /// Expiration time. `None` means permanent.
    #[serde(with = "option_epoch_serde")]
    pub expires_at: Option<SystemTime>,
    /// When the ban was created.
    #[serde(with = "epoch_serde")]
    pub created_at: SystemTime,
    /// Source of the ban (who banned: "console", "admin", plugin id, etc.).
    pub source: String,
}

impl BanEntry {
    /// Creates a new ban entry.
    pub fn new(
        target: BanTarget,
        reason: Option<String>,
        duration: Option<Duration>,
        source: String,
    ) -> Self {
        let now = SystemTime::now();
        Self {
            target,
            reason,
            expires_at: duration.map(|d| now + d),
            created_at: now,
            source,
        }
    }

    /// Returns `true` if this ban has expired.
    pub fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|exp| SystemTime::now() >= exp)
    }

    /// Returns `true` if this ban is permanent (no expiration).
    pub const fn is_permanent(&self) -> bool {
        self.expires_at.is_none()
    }

    /// Returns the remaining duration before expiration.
    /// `None` if permanent or already expired.
    pub fn remaining(&self) -> Option<Duration> {
        self.expires_at
            .and_then(|exp| exp.duration_since(SystemTime::now()).ok())
    }

    /// Builds the kick message shown to the player.
    pub fn kick_message(&self) -> String {
        let reason = self.reason.as_deref().unwrap_or("Banned by administrator");
        self.remaining().map_or_else(
            || format!("{reason}\n\nThis ban is permanent."),
            |remaining| {
                let hours = remaining.as_secs() / 3600;
                let minutes = (remaining.as_secs() % 3600) / 60;
                if hours > 24 {
                    let days = hours / 24;
                    format!("{reason}\n\nExpires in {days} day(s)")
                } else if hours > 0 {
                    format!("{reason}\n\nExpires in {hours}h {minutes}m")
                } else {
                    format!("{reason}\n\nExpires in {minutes} minute(s)")
                }
            },
        )
    }
}

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
