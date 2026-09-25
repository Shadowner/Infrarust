//! Ban service.

use std::fmt;
use std::net::IpAddr;
use std::time::{Duration, SystemTime};

pub use ipnet::IpNet;
use ipnet::Ipv4Net;
use uuid::Uuid;

use crate::error::ServiceError;
use crate::event::BoxFuture;
use crate::types::{Component, ServerId};

pub mod private {
    /// Sealed — only the proxy implements [`BanService`](super::BanService).
    pub trait Sealed {}
}

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
    Ip(IpAddr),
    IpRange(IpNet),
    Username(String),
    Uuid(Uuid),
}

impl BanTarget {
    pub const fn display_type(&self) -> &'static str {
        match self {
            Self::Ip(_) => "IP",
            Self::IpRange(_) => "IP range",
            Self::Username(_) => "username",
            Self::Uuid(_) => "UUID",
        }
    }

    #[must_use]
    pub fn canonical(self) -> Self {
        match self {
            Self::Ip(ip) => Self::Ip(ip.to_canonical()),
            Self::IpRange(net) => Self::IpRange(canonical_net(net)),
            other => other,
        }
    }

    pub fn matches_ip(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        match self {
            Self::Ip(banned) => banned.to_canonical() == ip,
            Self::IpRange(net) => canonical_net(*net).contains(&ip),
            _ => false,
        }
    }

    pub fn matches(&self, attempt: &LoginAttempt) -> bool {
        match self {
            Self::Ip(_) | Self::IpRange(_) => self.matches_ip(attempt.ip),
            Self::Username(name) => attempt
                .username
                .as_deref()
                .is_some_and(|username| username.to_lowercase() == name.to_lowercase()),
            Self::Uuid(uuid) => attempt.uuid == Some(*uuid),
        }
    }
}

fn canonical_net(net: IpNet) -> IpNet {
    if let IpNet::V6(v6) = net
        && v6.prefix_len() >= 96
        && let Some(v4) = v6.network().to_ipv4_mapped()
        && let Ok(mapped) = Ipv4Net::new(v4, v6.prefix_len() - 96)
    {
        return IpNet::V4(mapped.trunc());
    }
    net.trunc()
}

impl fmt::Display for BanTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ip(ip) => write!(f, "IP:{ip}"),
            Self::IpRange(net) => write!(f, "range:{net}"),
            Self::Username(name) => write!(f, "username:{name}"),
            Self::Uuid(uuid) => write!(f, "UUID:{uuid}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(tag = "type", content = "value", rename_all = "snake_case")
)]
#[non_exhaustive]
pub enum BanSource {
    Console,
    Player { uuid: Uuid, name: String },
    Plugin(String),
    WebApi { actor: Option<String> },
    System,
}

impl fmt::Display for BanSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Console => f.write_str("console"),
            Self::Player { name, .. } => write!(f, "player:{name}"),
            Self::Plugin(id) => write!(f, "plugin:{id}"),
            Self::WebApi { actor: None } => f.write_str("web-api"),
            Self::WebApi { actor: Some(actor) } => write!(f, "web-api:{actor}"),
            Self::System => f.write_str("system"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct BanEntry {
    pub id: String,
    pub target: BanTarget,
    pub reason: Option<String>,
    pub source: BanSource,
    #[cfg_attr(feature = "serde", serde(with = "epoch_serde"))]
    pub created_at: SystemTime,
    #[cfg_attr(feature = "serde", serde(with = "option_epoch_serde"))]
    pub expires_at: Option<SystemTime>,
}

impl BanEntry {
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

    pub fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|exp| SystemTime::now() >= exp)
    }

    pub const fn is_permanent(&self) -> bool {
        self.expires_at.is_none()
    }

    pub fn remaining(&self) -> Option<Duration> {
        self.expires_at
            .and_then(|exp| exp.duration_since(SystemTime::now()).ok())
    }

    pub fn default_kick_message(&self) -> Component {
        let reason = self.reason.as_deref().unwrap_or("Banned by administrator");
        let text = self.remaining().map_or_else(
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
        );
        Component::text(text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BanRequest {
    pub target: BanTarget,
    pub reason: Option<String>,
    pub duration: Option<Duration>,
    pub source: Option<BanSource>,
    pub kick: bool,
    pub silent: bool,
}

impl BanRequest {
    pub const fn new(target: BanTarget) -> Self {
        Self {
            target,
            reason: None,
            duration: None,
            source: None,
            kick: true,
            silent: false,
        }
    }

    #[must_use]
    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    #[must_use]
    pub const fn duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    #[must_use]
    pub fn source(mut self, source: BanSource) -> Self {
        self.source = Some(source);
        self
    }

    #[must_use]
    pub const fn kick(mut self, kick: bool) -> Self {
        self.kick = kick;
        self
    }

    #[must_use]
    pub const fn silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnbanRequest {
    pub target: BanTarget,
    pub source: Option<BanSource>,
    pub silent: bool,
}

impl UnbanRequest {
    pub const fn new(target: BanTarget) -> Self {
        Self {
            target,
            source: None,
            silent: false,
        }
    }

    #[must_use]
    pub fn source(mut self, source: BanSource) -> Self {
        self.source = Some(source);
        self
    }

    #[must_use]
    pub const fn silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }
}

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
    pub fn status(ip: IpAddr) -> Self {
        Self {
            stage: LoginStage::Status,
            ip: ip.to_canonical(),
            username: None,
            uuid: None,
            uuid_verified: false,
            virtual_host: None,
            server: None,
        }
    }

    pub fn pre_auth(ip: IpAddr, username: impl Into<String>) -> Self {
        Self {
            stage: LoginStage::PreAuth,
            username: Some(username.into()),
            ..Self::status(ip)
        }
    }

    pub fn post_auth(
        ip: IpAddr,
        username: impl Into<String>,
        uuid: Uuid,
        uuid_verified: bool,
    ) -> Self {
        Self {
            stage: LoginStage::PostAuth,
            username: Some(username.into()),
            uuid: Some(uuid),
            uuid_verified,
            ..Self::status(ip)
        }
    }

    #[must_use]
    pub const fn claimed_uuid(mut self, uuid: Option<Uuid>) -> Self {
        self.uuid = uuid;
        self.uuid_verified = false;
        self
    }

    #[must_use]
    pub fn virtual_host(mut self, host: impl Into<String>) -> Self {
        self.virtual_host = Some(host.into());
        self
    }

    #[must_use]
    pub fn server(mut self, server: ServerId) -> Self {
        self.server = Some(server);
        self
    }

    pub fn is_login(&self) -> bool {
        self.stage != LoginStage::Status
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BanVerdict {
    pub entry: BanEntry,
    pub kick_message: Component,
}

impl BanVerdict {
    pub fn new(entry: BanEntry) -> Self {
        let kick_message = entry.default_kick_message();
        Self {
            entry,
            kick_message,
        }
    }

    #[must_use]
    pub fn message(mut self, message: Component) -> Self {
        self.kick_message = message;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BanQuery {
    pub cursor: Option<String>,
    pub limit: usize,
}

impl BanQuery {
    pub const DEFAULT_LIMIT: usize = 100;
    pub const MAX_LIMIT: usize = 1000;

    pub const fn new() -> Self {
        Self {
            cursor: None,
            limit: Self::DEFAULT_LIMIT,
        }
    }

    #[must_use]
    pub fn after(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }

    #[must_use]
    pub const fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    pub fn effective_limit(&self) -> usize {
        self.limit.clamp(1, Self::MAX_LIMIT)
    }
}

impl Default for BanQuery {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct BanPage {
    pub entries: Vec<BanEntry>,
    pub next_cursor: Option<String>,
}

impl BanPage {
    pub const fn new(entries: Vec<BanEntry>, next_cursor: Option<String>) -> Self {
        Self {
            entries,
            next_cursor,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct BanFeatures {
    pub ip_ranges: bool,
    pub pagination: bool,
}

impl BanFeatures {
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
}

pub trait BanProvider: Send + Sync {
    fn check<'a>(
        &'a self,
        attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>>;

    fn ban(&self, request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>>;

    fn unban(&self, request: UnbanRequest)
    -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>>;

    fn get<'a>(
        &'a self,
        target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>>;

    fn list(&self, query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>>;

    fn features(&self) -> BanFeatures {
        BanFeatures::new()
    }
}

pub trait BanService: Send + Sync + private::Sealed {
    fn check<'a>(
        &'a self,
        attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>>;

    fn ban(&self, request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>>;

    fn unban(&self, request: UnbanRequest)
    -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>>;

    fn get<'a>(
        &'a self,
        target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>>;

    fn list(&self, query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>>;

    fn features(&self) -> BanFeatures;

    fn list_all(&self) -> BoxFuture<'_, Result<Vec<BanEntry>, ServiceError>> {
        Box::pin(async move {
            let mut entries = Vec::new();
            let mut query = BanQuery::new().limit(BanQuery::MAX_LIMIT);
            loop {
                let page = self.list(query.clone()).await?;
                entries.extend(page.entries);
                match page.next_cursor {
                    Some(next) if query.cursor.as_deref() != Some(next.as_str()) => {
                        query = query.after(next);
                    }
                    _ => return Ok(entries),
                }
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BanProviderRejected {
    #[error("the plugin lacks the ban-provider capability")]
    MissingCapability,
    #[error("[ban] provider selects `{selected}`, not this plugin")]
    NotSelected { selected: String },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn range(s: &str) -> BanTarget {
        BanTarget::IpRange(s.parse().unwrap())
    }

    #[test]
    fn ban_target_non_exhaustive() {
        let target = BanTarget::Username("griefer".into());
        #[allow(unreachable_patterns)]
        match target {
            BanTarget::Ip(_)
            | BanTarget::IpRange(_)
            | BanTarget::Username(_)
            | BanTarget::Uuid(_)
            | _ => {}
        }
    }

    #[test]
    fn ban_entry_permanent() {
        let entry = BanEntry::new(
            "1",
            BanTarget::Username("griefer".into()),
            BanSource::Console,
        )
        .reason("griefing");
        assert!(entry.is_permanent());
        assert!(!entry.is_expired());
        assert!(entry.remaining().is_none());
        assert!(
            entry
                .default_kick_message()
                .to_plain()
                .contains("permanent")
        );
    }

    #[test]
    fn ban_entry_temporary() {
        let entry = BanEntry::new("1", BanTarget::Ip(ip("127.0.0.1")), BanSource::System)
            .lasting(Duration::from_secs(3600));
        assert!(!entry.is_permanent());
        assert!(!entry.is_expired());
        assert!(entry.remaining().is_some());
    }

    #[test]
    fn ban_entry_overflowing_duration_is_permanent() {
        let entry = BanEntry::new(
            "1",
            BanTarget::Username("griefer".into()),
            BanSource::Console,
        )
        .lasting(Duration::MAX);
        assert!(entry.is_permanent());
        assert!(!entry.is_expired());
    }

    #[test]
    fn ban_target_display() {
        assert_eq!(BanTarget::Ip(ip("1.2.3.4")).to_string(), "IP:1.2.3.4");
        assert_eq!(range("10.0.0.0/8").to_string(), "range:10.0.0.0/8");
        assert_eq!(
            BanTarget::Username("test".into()).to_string(),
            "username:test"
        );
        assert_eq!(BanTarget::Ip(ip("1.2.3.4")).display_type(), "IP");
        assert_eq!(range("10.0.0.0/8").display_type(), "IP range");
    }

    #[test]
    fn an_ipv4_range_holds_the_addresses_inside_it_and_nothing_else() {
        let net = range("10.20.0.0/16");
        assert!(net.matches_ip(ip("10.20.0.1")));
        assert!(net.matches_ip(ip("10.20.255.254")));
        assert!(!net.matches_ip(ip("10.21.0.1")));
        assert!(!net.matches_ip(ip("::1")));
    }

    #[test]
    fn an_ipv6_range_holds_the_addresses_inside_it_and_nothing_else() {
        let net = range("2001:db8:abcd::/48");
        assert!(net.matches_ip(ip("2001:db8:abcd:12::1")));
        assert!(!net.matches_ip(ip("2001:db8:abce::1")));
        assert!(!net.matches_ip(ip("10.0.0.1")));
    }

    #[test]
    fn a_v4_mapped_client_address_hits_the_ipv4_range_and_the_ipv4_ban() {
        let mapped = ip("::ffff:10.20.3.4");
        assert!(range("10.20.0.0/16").matches_ip(mapped));
        assert!(BanTarget::Ip(ip("10.20.3.4")).matches_ip(mapped));
        assert!(!range("10.21.0.0/16").matches_ip(mapped));
    }

    #[test]
    fn a_v4_mapped_range_holds_plain_ipv4_clients() {
        let net = range("::ffff:10.20.0.0/112");
        assert!(net.matches_ip(ip("10.20.9.9")));
        assert!(net.matches_ip(ip("::ffff:10.20.9.9")));
        assert!(!net.matches_ip(ip("10.21.0.1")));
        assert_eq!(net.canonical(), range("10.20.0.0/16"));
    }

    #[test]
    fn a_range_with_host_bits_is_stored_as_its_network() {
        assert_eq!(range("10.20.3.4/16").canonical(), range("10.20.0.0/16"));
        assert_eq!(
            BanTarget::Ip(ip("::ffff:1.2.3.4")).canonical(),
            BanTarget::Ip(ip("1.2.3.4"))
        );
    }

    #[test]
    fn a_target_matches_the_attempt_it_names() {
        let uuid = Uuid::new_v4();
        let attempt = LoginAttempt::post_auth(ip("::ffff:192.0.2.5"), "Steve", uuid, true);
        assert!(BanTarget::Ip(ip("192.0.2.5")).matches(&attempt));
        assert!(range("192.0.2.0/24").matches(&attempt));
        assert!(BanTarget::Username("sTEVE".into()).matches(&attempt));
        assert!(BanTarget::Uuid(uuid).matches(&attempt));
        assert!(!BanTarget::Uuid(Uuid::new_v4()).matches(&attempt));
        assert!(!BanTarget::Username("Alex".into()).matches(&attempt));
        let status = LoginAttempt::status(ip("192.0.2.5"));
        assert!(!BanTarget::Username("Steve".into()).matches(&status));
        assert!(range("192.0.2.0/24").matches(&status));
    }

    #[test]
    fn sources_render_who_acted() {
        assert_eq!(BanSource::Console.to_string(), "console");
        assert_eq!(
            BanSource::Plugin("libertybans".into()).to_string(),
            "plugin:libertybans"
        );
        assert_eq!(BanSource::WebApi { actor: None }.to_string(), "web-api");
        assert_eq!(
            BanSource::Player {
                uuid: Uuid::nil(),
                name: "Mod".into()
            }
            .to_string(),
            "player:Mod"
        );
    }

    #[test]
    fn a_query_limit_is_kept_inside_its_bounds() {
        assert_eq!(BanQuery::new().limit(0).effective_limit(), 1);
        assert_eq!(
            BanQuery::new().limit(usize::MAX).effective_limit(),
            BanQuery::MAX_LIMIT
        );
        assert_eq!(BanQuery::new().effective_limit(), BanQuery::DEFAULT_LIMIT);
    }
}
