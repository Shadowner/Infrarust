use std::net::IpAddr;
use std::time::{Duration, SystemTime};

use uuid::Uuid;

use crate::bindings::ban_service as wb;
use crate::component::Component;
use crate::error::PluginError;
use crate::event::BanSource;
use crate::services::{BanRequest, BanTarget};
use crate::types::{ServerId, ip_from_wit, millis_since_epoch, uuid_from_wit};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LoginStage {
    Status,
    PreAuth,
    PostAuth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LoginAttempt {
    pub stage: LoginStage,
    pub ip: IpAddr,
    pub username: Option<String>,
    pub uuid: Option<Uuid>,
    pub uuid_verified: bool,
    pub virtual_host: Option<String>,
    pub server: Option<ServerId>,
}

impl LoginAttempt {
    #[must_use]
    pub fn is_login(&self) -> bool {
        self.stage != LoginStage::Status
    }

    pub(crate) fn from_wit(attempt: wb::LoginAttempt) -> Self {
        Self {
            stage: match attempt.stage {
                wb::LoginStage::Status => LoginStage::Status,
                wb::LoginStage::PreAuth => LoginStage::PreAuth,
                wb::LoginStage::PostAuth => LoginStage::PostAuth,
            },
            ip: ip_from_wit(attempt.ip),
            username: attempt.username,
            uuid: attempt.uuid.map(uuid_from_wit),
            uuid_verified: attempt.uuid_verified,
            virtual_host: attempt.virtual_host,
            server: attempt.server.map(ServerId::from),
        }
    }
}

impl BanTarget {
    #[must_use]
    pub fn matches(&self, attempt: &LoginAttempt) -> bool {
        match self {
            Self::Ip(ip) => ip.to_canonical() == attempt.ip.to_canonical(),
            Self::IpRange(range) => in_range(range, attempt.ip),
            Self::Username(name) => attempt
                .username
                .as_deref()
                .is_some_and(|username| username.eq_ignore_ascii_case(name)),
            Self::Uuid(uuid) => attempt.uuid == Some(*uuid),
        }
    }
}

fn in_range(range: &str, ip: IpAddr) -> bool {
    let Some((network, prefix)) = range.split_once('/') else {
        return range
            .parse::<IpAddr>()
            .is_ok_and(|single| single.to_canonical() == ip.to_canonical());
    };
    let (Ok(network), Ok(prefix)) = (network.parse::<IpAddr>(), prefix.parse::<u32>()) else {
        return false;
    };
    match (network.to_canonical(), ip.to_canonical()) {
        (IpAddr::V4(network), IpAddr::V4(ip)) if prefix <= 32 => {
            let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
            u32::from(network) & mask == u32::from(ip) & mask
        }
        (IpAddr::V6(network), IpAddr::V6(ip)) if prefix <= 128 => {
            let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
            u128::from(network) & mask == u128::from(ip) & mask
        }
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BanRecord {
    pub id: String,
    pub target: BanTarget,
    pub reason: Option<String>,
    pub source: BanSource,
    pub created_at: SystemTime,
    pub expires_at: Option<SystemTime>,
}

impl BanRecord {
    #[must_use]
    pub fn new(id: impl Into<String>, target: BanTarget, source: BanSource) -> Self {
        Self {
            id: id.into(),
            target,
            reason: None,
            source,
            created_at: SystemTime::now(),
            expires_at: None,
        }
    }

    #[must_use]
    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    #[must_use]
    pub const fn created_at(mut self, at: SystemTime) -> Self {
        self.created_at = at;
        self
    }

    #[must_use]
    pub const fn expires_at(mut self, at: SystemTime) -> Self {
        self.expires_at = Some(at);
        self
    }

    #[must_use]
    pub fn lasting(mut self, duration: Duration) -> Self {
        self.expires_at = self.created_at.checked_add(duration);
        self
    }

    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|at| SystemTime::now() >= at)
    }

    #[must_use]
    pub const fn is_permanent(&self) -> bool {
        self.expires_at.is_none()
    }

    pub(crate) fn to_wit(&self) -> wb::BanRecord {
        wb::BanRecord {
            id: self.id.clone(),
            target: self.target.to_wit(),
            reason: self.reason.clone(),
            source: self.source.to_wit(),
            created_at: millis_since_epoch(self.created_at),
            expires_at: self.expires_at.map(millis_since_epoch),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BanVerdict {
    pub entry: BanRecord,
    pub message: Option<Component>,
}

impl BanVerdict {
    #[must_use]
    pub const fn new(entry: BanRecord) -> Self {
        Self {
            entry,
            message: None,
        }
    }

    #[must_use]
    pub fn message(mut self, message: impl Into<Component>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub(crate) fn to_wit(&self) -> wb::BanVerdict {
        wb::BanVerdict {
            entry: self.entry.to_wit(),
            kick_message: self.message.as_ref().map(Component::to_arena),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnbanRequest {
    pub target: BanTarget,
    pub source: BanSource,
    pub silent: bool,
}

impl UnbanRequest {
    pub(crate) fn from_wit(request: wb::UnbanRequest) -> Self {
        Self {
            target: BanTarget::from_wit(request.target),
            source: BanSource::from_wit(request.source),
            silent: request.silent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BanQuery {
    pub cursor: Option<String>,
    pub limit: u32,
}

impl BanQuery {
    pub(crate) fn from_wit(query: wb::BanQuery) -> Self {
        Self {
            cursor: query.cursor,
            limit: query.limit,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct BanRecordPage {
    pub entries: Vec<BanRecord>,
    pub next_cursor: Option<String>,
}

impl BanRecordPage {
    #[must_use]
    pub const fn new(entries: Vec<BanRecord>, next_cursor: Option<String>) -> Self {
        Self {
            entries,
            next_cursor,
        }
    }

    pub(crate) fn to_wit(&self) -> wb::BanRecordPage {
        wb::BanRecordPage {
            entries: self.entries.iter().map(BanRecord::to_wit).collect(),
            next_cursor: self.next_cursor.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct BanFeatures {
    pub ip_ranges: bool,
    pub pagination: bool,
}

impl BanFeatures {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ip_ranges: false,
            pagination: false,
        }
    }

    #[must_use]
    pub const fn ip_ranges(mut self, supported: bool) -> Self {
        self.ip_ranges = supported;
        self
    }

    #[must_use]
    pub const fn pagination(mut self, supported: bool) -> Self {
        self.pagination = supported;
        self
    }

    pub(crate) const fn to_wit(self) -> wb::BanFeatures {
        wb::BanFeatures {
            ip_ranges: self.ip_ranges,
            pagination: self.pagination,
        }
    }
}

pub trait BanProvider {
    fn check(&self, attempt: &LoginAttempt) -> Result<Option<BanVerdict>, PluginError>;

    fn ban(&self, request: BanRequest, source: BanSource) -> Result<BanRecord, PluginError>;

    fn unban(&self, request: UnbanRequest) -> Result<Option<BanRecord>, PluginError>;

    fn get(&self, target: &BanTarget) -> Result<Option<BanRecord>, PluginError>;

    fn list(&self, query: &BanQuery) -> Result<BanRecordPage, PluginError>;

    fn features(&self) -> BanFeatures {
        BanFeatures::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::time_from_millis;

    fn record_from_wit(record: wb::BanRecord) -> BanRecord {
        BanRecord {
            id: record.id,
            target: BanTarget::from_wit(record.target),
            reason: record.reason,
            source: BanSource::from_wit(record.source),
            created_at: time_from_millis(record.created_at),
            expires_at: record.expires_at.map(time_from_millis),
        }
    }

    fn attempt(ip: &str, username: Option<&str>, uuid: Option<Uuid>) -> LoginAttempt {
        LoginAttempt {
            stage: LoginStage::PostAuth,
            ip: ip.parse().unwrap(),
            username: username.map(str::to_owned),
            uuid,
            uuid_verified: true,
            virtual_host: None,
            server: None,
        }
    }

    #[test]
    fn a_target_matches_the_attempt_it_names() {
        let uuid = Uuid::from_u128(9);
        let steve = attempt("::ffff:192.0.2.5", Some("Steve"), Some(uuid));
        assert!(BanTarget::Ip("192.0.2.5".parse().unwrap()).matches(&steve));
        assert!(BanTarget::IpRange("192.0.2.0/24".into()).matches(&steve));
        assert!(!BanTarget::IpRange("192.0.3.0/24".into()).matches(&steve));
        assert!(BanTarget::IpRange("0.0.0.0/0".into()).matches(&steve));
        assert!(BanTarget::Username("sTEVE".into()).matches(&steve));
        assert!(BanTarget::Uuid(uuid).matches(&steve));
        assert!(!BanTarget::Uuid(Uuid::from_u128(1)).matches(&steve));
        assert!(!BanTarget::IpRange("nonsense".into()).matches(&steve));

        let v6 = attempt("2001:db8:abcd::1", None, None);
        assert!(BanTarget::IpRange("2001:db8:abcd::/48".into()).matches(&v6));
        assert!(!BanTarget::IpRange("2001:db8:abce::/48".into()).matches(&v6));
        assert!(!BanTarget::Username("Steve".into()).matches(&v6));
    }

    #[test]
    fn a_record_crosses_the_boundary_intact() {
        let created = time_from_millis(1_700_000_000_000);
        let record = BanRecord::new(
            "b1",
            BanTarget::Username("Steve".into()),
            BanSource::Player {
                uuid: Uuid::from_u128(3),
                name: "Mod".into(),
            },
        )
        .reason("griefing")
        .created_at(created)
        .lasting(Duration::from_secs(60));
        assert!(!record.is_permanent());
        assert_eq!(record_from_wit(record.to_wit()), record);
        assert_eq!(millis_since_epoch(created), 1_700_000_000_000);

        let verdict = BanVerdict::new(record.clone()).message("go away");
        let wit = verdict.to_wit();
        assert!(wit.kick_message.is_some());
        assert_eq!(BanVerdict::new(record).to_wit().kick_message, None);
    }
}
