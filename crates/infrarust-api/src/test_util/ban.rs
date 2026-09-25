use std::sync::Mutex;

use crate::error::ServiceError;
use crate::event::BoxFuture;
use crate::services::ban_service::{
    BanEntry, BanFeatures, BanPage, BanQuery, BanRequest, BanService, BanSource, BanTarget,
    BanVerdict, LoginAttempt, UnbanRequest,
};

use super::lock;

#[derive(Debug, Default)]
struct State {
    entries: Vec<BanEntry>,
    next_id: u64,
    checks: Vec<LoginAttempt>,
    unavailable: bool,
}

#[derive(Debug, Default)]
pub struct MockBanService {
    state: Mutex<State>,
}

impl MockBanService {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_entry(self, entry: BanEntry) -> Self {
        self.insert(entry);
        self
    }

    pub fn insert(&self, entry: BanEntry) {
        let mut state = lock(&self.state);
        state.entries.retain(|e| e.target != entry.target);
        state.entries.push(entry);
    }

    #[must_use]
    pub fn entries(&self) -> Vec<BanEntry> {
        lock(&self.state).entries.clone()
    }

    #[must_use]
    pub fn checks(&self) -> Vec<LoginAttempt> {
        lock(&self.state).checks.clone()
    }

    pub fn set_unavailable(&self, unavailable: bool) {
        lock(&self.state).unavailable = unavailable;
    }

    fn available(&self) -> Result<std::sync::MutexGuard<'_, State>, ServiceError> {
        let state = lock(&self.state);
        if state.unavailable {
            return Err(ServiceError::Unavailable("mock ban service".into()));
        }
        Ok(state)
    }
}

impl crate::services::ban_service::private::Sealed for MockBanService {}

impl BanService for MockBanService {
    fn check<'a>(
        &'a self,
        attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>> {
        let result = self.available().map(|mut state| {
            state.checks.push(attempt.clone());
            state
                .entries
                .iter()
                .find(|e| !e.is_expired() && e.target.matches(attempt))
                .cloned()
                .map(BanVerdict::new)
        });
        Box::pin(async move { result })
    }

    fn ban(&self, request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>> {
        let result = self.available().map(|mut state| {
            state.next_id += 1;
            let source = request.source.clone().unwrap_or(BanSource::System);
            let mut entry = BanEntry::new(
                state.next_id.to_string(),
                request.target.clone().canonical(),
                source,
            );
            if let Some(reason) = &request.reason {
                entry = entry.reason(reason.clone());
            }
            if let Some(duration) = request.duration {
                entry = entry.lasting(duration);
            }
            state.entries.retain(|e| e.target != entry.target);
            state.entries.push(entry.clone());
            entry
        });
        Box::pin(async move { result })
    }

    fn unban(
        &self,
        request: UnbanRequest,
    ) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        let target = request.target.canonical();
        let result = self.available().map(|mut state| {
            let at = state.entries.iter().position(|e| e.target == target)?;
            Some(state.entries.remove(at))
        });
        Box::pin(async move { result })
    }

    fn get<'a>(
        &'a self,
        target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>> {
        let target = target.clone().canonical();
        let result = self.available().map(|state| {
            state
                .entries
                .iter()
                .find(|e| e.target == target && !e.is_expired())
                .cloned()
        });
        Box::pin(async move { result })
    }

    fn list(&self, query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>> {
        let result = self.available().map(|state| {
            let start = query
                .cursor
                .as_deref()
                .and_then(|cursor| state.entries.iter().position(|e| e.id == cursor))
                .map_or(0, |at| at + 1);
            let limit = query.effective_limit();
            let entries: Vec<BanEntry> = state
                .entries
                .iter()
                .skip(start)
                .take(limit)
                .cloned()
                .collect();
            let next_cursor = (start + entries.len() < state.entries.len())
                .then(|| entries.last().map(|e| e.id.clone()))
                .flatten();
            BanPage::new(entries, next_cursor)
        });
        Box::pin(async move { result })
    }

    fn features(&self) -> BanFeatures {
        BanFeatures::new().ip_ranges(true).pagination(true)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::net::IpAddr;

    use super::*;
    use crate::test_util::block_on;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn a_ban_matches_later_logins_until_revoked() {
        let bans = MockBanService::new();
        let entry = block_on(
            bans.ban(BanRequest::new(BanTarget::Username("Griefer".into())).reason("grief")),
        )
        .unwrap();
        assert_eq!(entry.reason.as_deref(), Some("grief"));

        let attempt = LoginAttempt::pre_auth(ip("1.2.3.4"), "griefer");
        let verdict = block_on(bans.check(&attempt)).unwrap().unwrap();
        assert_eq!(verdict.entry.id, entry.id);
        assert_eq!(bans.checks(), std::slice::from_ref(&attempt));

        let revoked =
            block_on(bans.unban(UnbanRequest::new(BanTarget::Username("Griefer".into())))).unwrap();
        assert!(revoked.is_some());
        assert!(block_on(bans.check(&attempt)).unwrap().is_none());
    }

    #[test]
    fn ranges_match_and_pages_follow_the_cursor() {
        let bans = MockBanService::new();
        for n in 0..3 {
            block_on(bans.ban(BanRequest::new(BanTarget::Ip(ip(&format!("10.0.0.{n}")))))).unwrap();
        }
        block_on(bans.ban(BanRequest::new(BanTarget::IpRange(
            "192.168.0.0/16".parse().unwrap(),
        ))))
        .unwrap();
        let attempt = LoginAttempt::status(ip("192.168.4.4"));
        assert!(block_on(bans.check(&attempt)).unwrap().is_some());

        let first = block_on(bans.list(BanQuery::new().limit(3))).unwrap();
        assert_eq!(first.entries.len(), 3);
        let cursor = first.next_cursor.unwrap();
        let second = block_on(bans.list(BanQuery::new().limit(3).after(cursor))).unwrap();
        assert_eq!(second.entries.len(), 1);
        assert!(second.next_cursor.is_none());
        assert_eq!(block_on(bans.list_all()).unwrap().len(), 4);
    }

    #[test]
    fn an_unavailable_service_fails_every_call() {
        let bans = MockBanService::new();
        bans.set_unavailable(true);
        let attempt = LoginAttempt::status(ip("1.1.1.1"));
        assert!(matches!(
            block_on(bans.check(&attempt)),
            Err(ServiceError::Unavailable(_))
        ));
    }
}
