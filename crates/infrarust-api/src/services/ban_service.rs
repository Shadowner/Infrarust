//! Ban service.

use std::fmt;
use std::net::IpAddr;
use std::time::{Duration, SystemTime};

use crate::error::ServiceError;
use crate::event::BoxFuture;

pub mod private {
    /// Sealed — only the proxy implements [`BanService`](super::BanService).
    pub trait Sealed {}
}

/// The target of a ban.
#[cfg(feature = "serde")]
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
#[cfg(feature = "serde")]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(tag = "type", content = "value", rename_all = "snake_case")
)]
#[non_exhaustive]
pub enum BanTarget {
    /// Ban by exact IP address (no CIDR).
    Ip(IpAddr),
    /// Ban by Minecraft username (case-insensitive for lookups).
    Username(String),
    /// Ban by Mojang UUID.
    Uuid(uuid::Uuid),
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
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BanEntry {
    /// The ban target.
    pub target: BanTarget,
    /// Reason for the ban (shown to the player on kick).
    pub reason: Option<String>,
    /// Expiration time. `None` means permanent.
    #[cfg_attr(feature = "serde", serde(with = "option_epoch_serde"))]
    pub expires_at: Option<SystemTime>,
    /// When the ban was created.
    #[cfg_attr(feature = "serde", serde(with = "epoch_serde"))]
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

// ─────────────────────────── BanService ──────────────────────────

/// Service for managing player bans.
///
/// Obtained via [`PluginContext::ban_service()`](crate::plugin::PluginContext::ban_service).
pub trait BanService: Send + Sync + private::Sealed {
    /// Bans a target with an optional reason and duration.
    ///
    /// A `None` duration means permanent ban.
    fn ban(
        &self,
        target: BanTarget,
        reason: Option<String>,
        duration: Option<Duration>,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;

    /// Removes a ban. Returns `true` if a ban was removed.
    fn unban(&self, target: &BanTarget) -> BoxFuture<'_, Result<bool, ServiceError>>;

    /// Checks if a target is currently banned.
    fn is_banned(&self, target: &BanTarget) -> BoxFuture<'_, Result<bool, ServiceError>>;

    /// Returns the ban entry for a target, if any.
    fn get_ban(&self, target: &BanTarget) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>>;

    /// Returns all active bans.
    fn get_all_bans(&self) -> BoxFuture<'_, Result<Vec<BanEntry>, ServiceError>>;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn ban_target_non_exhaustive() {
        let target = BanTarget::Username("griefer".into());
        #[allow(unreachable_patterns)]
        match target {
            BanTarget::Ip(_) | BanTarget::Username(_) | BanTarget::Uuid(_) | _ => {}
        }
    }

    #[test]
    fn ban_entry_permanent() {
        let entry = BanEntry::new(
            BanTarget::Username("griefer".into()),
            Some("griefing".into()),
            None,
            "admin".into(),
        );
        assert!(entry.is_permanent());
        assert!(!entry.is_expired());
        assert!(entry.remaining().is_none());
        assert!(entry.kick_message().contains("permanent"));
    }

    #[test]
    fn ban_entry_temporary() {
        let entry = BanEntry::new(
            BanTarget::Ip("127.0.0.1".parse().unwrap()),
            None,
            Some(Duration::from_secs(3600)),
            "system".into(),
        );
        assert!(!entry.is_permanent());
        assert!(!entry.is_expired());
        assert!(entry.remaining().is_some());
    }

    #[test]
    fn ban_target_display() {
        assert_eq!(
            BanTarget::Ip("1.2.3.4".parse().unwrap()).to_string(),
            "IP:1.2.3.4"
        );
        assert_eq!(
            BanTarget::Username("test".into()).to_string(),
            "username:test"
        );
        assert_eq!(BanTarget::Ip("1.2.3.4".parse().unwrap()).display_type(), "IP");
    }
}
